import { BrowserRuntimeBudget, sharedBrowserRuntimeBudget, type RuntimePriority } from "./runtime-budget.ts";
import { PreviewBuffer } from "./compositor/preview-buffer.ts";
import { waitForGpuCompletion } from "../executor/webgl/gpu-completion.ts";
import type { RenderTrace } from "./render-trace.ts";
import { MediaBlobCache } from "../media/media-blob-cache.ts";
import { createMotionFontLoader, type MotionFontSource } from "../fonts.ts";
import canvasKitPackage from "canvaskit-wasm/package.json";
import {
  CanvasKitScene3dDof,
  decodeDeferredScene3dDof,
} from "../executor/canvaskit/scene3d-dof.ts";
import { createScene3dGpu } from "../executor/webgl/scene3d-factory.ts";
import type {
  GpuScene3D,
  Scene3dGpuRequest,
} from "../executor/webgl/scene3d.js";
import { GpuCircles } from "../executor/webgl/circles.js";
import {
  resetCanvasKitContext,
  untrackGpuState,
} from "../executor/webgl/gpu-state.ts";
// Browser Product Compositor host.
//
// Rust/WASM owns Timeline evaluation, Motion/caption preparation, RenderGraph construction and
// lowering. This file owns only browser resources, clock/scheduling, an opaque generation-scoped
// object table, CanvasKit execution and presentation.

import {
  RESOURCE_REQUESTS_ABI,
  BOUND_PROGRAM_SCHEDULES_ABI,
  decodePackedAbi,
  type PackedValue,
} from "../abi/packed.ts";
import {
  CanvasKitExecutor,
  type CanvasKitExecutionProfile,
  type CanvasKitExternalObject,
} from "../executor/canvaskit/index.ts";
import type {
  ResourceManifest,
  TimelineDocument,
} from "../internal-timeline.ts";
import {
  createDecoderRing,
  keyframeThumbnails,
  type DecoderRing,
} from "../media/video-source.ts";
import { BrowserAudioWindowReader } from "../media/audio-source.ts";
import {
  resolvePlayerRuntimeAssets,
  type PlayerRuntimeAssets,
} from "../runtime-assets.ts";
import { canonicalizeTimelineDocumentWithWasm } from "../compiler.ts";
import {
  ProductFramePlannerPool,
  type ProductFramePlanningInput,
  type ProductFramePlanningJob,
} from "./compositor/frame-pool.ts";
import {
  emptyProductEngineBootstrap,
  parseProductRenderReceipt,
  type ProductEngineBootstrap,
  type ProductEngineWire,
  type ProductRenderReceipt,
  type ProductRuntimeSnapshot,
  type ProductRuntimeMethod,
} from "./compositor/frame-protocol.ts";
import { assertCanvaskitImportLayout } from "./visual-layout.ts";
import {
  BrowserResourceCache,
  resourceRequestCacheIdentity,
  type BrowserResourceCacheFrameReport,
  type BrowserResourceCacheLimits,
} from "./resource-cache.ts";
import type CanvasKitInit from "canvaskit-wasm";
import type {
  CanvasKit,
  GrDirectContext,
  Image,
  PartialImageInfo,
  Surface,
} from "canvaskit-wasm";

type CanvasKitInitializer = typeof CanvasKitInit;
type Wire = Record<string, any>;

/** Platform locator only. Resource kind, digest and descriptor come from ResourceManifest. */
export interface BrowserResourceLocator {
  id: string;
  url: string;
}

interface AdmittedBrowserAsset extends BrowserResourceLocator {
  type: string;
  contentDigest: string;
  descriptor: Record<string, unknown>;
}

export interface BrowserValleWebPlayerOptions {
  fixedPackageManifestJson: string;
  timelineJson: string;
  resourceManifestJson: string;
  verifiedBindingBundleJson: string;
  assets?: BrowserResourceLocator[];
  /** Replace ordinary Motion fonts with this ordered, lazily loaded stack. Omit to use the work's fonts. */
  fonts?: readonly MotionFontSource[];
  canvas?: HTMLCanvasElement | null;
  assetBaseUrl?: string;
  proxyBase?: string | null;
  runtimeAssets: unknown;
  runtimeBaseUrl?: string | URL;
  pxScale?: number;
  gpu?: boolean;
  /** CPU task allowance (1..8); does not pin or reserve physical cores. */
  cpuThreads?: number;
  memoryBytes?: number;
  runtimeBudget?: BrowserRuntimeBudget;
  runtimePurpose?: "preview" | "export";
  /** Select accelerated preview stages, or retain the reference export path. */
  renderMode?: "reference" | "realtime";
  /** Optional bounded CPU trace; capture readbacks and GPU device time are separate. */
  trace?: RenderTrace;
  audioContext?: AudioContext | null;
  resourceCache?: BrowserResourceCacheLimits;
}

export type RenderPackageReplacement = Pick<
  BrowserValleWebPlayerOptions,
  | "fixedPackageManifestJson"
  | "timelineJson"
  | "resourceManifestJson"
  | "verifiedBindingBundleJson"
> &
  Partial<Pick<BrowserValleWebPlayerOptions, "assets">> & {
    isCurrent?: () => boolean;
  };

export interface CanonicalTimelineDocument {
  timeline: TimelineDocument;
  timelineJson: string;
  view: TimelineDocumentView;
}

export interface TimelineDocumentView {
  canvas: {
    width: number;
    height: number;
    durationSeconds: number;
    framesPerSecond: number;
    frameCount: number;
    sampleRate: number;
    sampleCount: number;
  };
  sequences: TimelineSequenceView[];
}

export interface TimelineSequenceView {
  band: "visual" | "audio" | "caption" | "adjustment";
  trackId: string;
  trackIndex: number;
  durationSeconds: number;
  items: Array<{
    itemId: string;
    /** JSON Pointer into the sparse public Timeline, or null for compiler-generated items. */
    timelinePath: string | null;
    itemIndex: number;
    startSeconds: number;
    durationSeconds: number;
    endSeconds: number;
    startFrame: number;
    durationFrames: number;
    endFrame: number;
    advancesCursor: boolean;
    sourceStartSeconds: number | null;
    sourceStartFrame: number | null;
    sourceRate: number | null;
  }>;
}

interface RenderOptions {
  png?: boolean;
  frame?: number;
  rasterScale?: number;
  transparent?: boolean;
}

interface BrowserProductEngineWire extends ProductEngineWire {
  decode_shader_data_texture(
    digest: string,
    encoded: Uint8Array,
    width: number,
    height: number,
  ): Uint8Array;
  register_scene3d_model(digest: string, bytes: Uint8Array): void;
  register_scene3d_environment(digest: string, bytes: Uint8Array): void;
  scene3d_resource_needs_json(canonicalRequest: Uint8Array): string;
  register_scene3d_texture(
    digest: string,
    encodedBytes: Uint8Array,
    role: "color" | "data",
  ): void;
  render_scene3d_request(
    contentDigest: string,
    topologyDigest: string,
    canonicalRequest: Uint8Array,
  ): Uint8Array;
  render_scene3d_request_deferred?(
    contentDigest: string,
    topologyDigest: string,
    canonicalRequest: Uint8Array,
  ): Uint8Array;
  admit_scene3d_gpu_request?(
    contentDigest: string,
    topologyDigest: string,
    canonicalRequest: Uint8Array,
  ): void;
  scene3d_gpu_lighting_input?(request: Uint8Array): Float32Array;
  has_scene3d_frame?(contentDigest: string): boolean;
  scene3d_frame_width(contentDigest: string): number;
  scene3d_frame_height(contentDigest: string): number;
  scene3d_pick_json(
    contentDigest: string,
    pixelX: number,
    pixelY: number,
  ): string;
  reset(): void;
}

interface ValleWasmModule {
  with_motion_fonts(
    packageJson: string,
    timelineJson: string,
    manifestJson: string,
    bundleJson: string,
    fontsJson: string,
  ): string;
  default(wasmUrl: string): Promise<void>;
  canonicalize_timeline_document(timelineJson: string): string;
  timeline_document_view(timelineJson: string): string;
  ProductEngine: new () => BrowserProductEngineWire;
}

interface CompiledAudioProgramWire {
  tracks: unknown[];
  sampleRate: number;
  sampleCount: number;
}

interface CompiledAudioPlayback {
  buffer: AudioBuffer;
  startSample: number;
}

interface AudioSourceRequirement {
  digest: ContentDigestWire;
  sourceChannels: 1 | 2;
  startSample: number;
  endSample: number;
}

interface AudioGraph {
  master: GainNode;
  sources: AudioBufferSourceNode[];
  nextSample: number;
  scheduledUntilContextTime: number;
}

interface TargetSurface {
  key: string;
  surface: Surface;
  gpu: boolean;
  context: GrDirectContext | null;
  canvas: HTMLCanvasElement | null;
}

interface FrozenBundleResources {
  environments: Map<string, Uint8Array>;
  fonts: Map<string, Uint8Array>;
  models: Map<string, Uint8Array>;
  shaders: Map<string, Uint8Array>;
}

interface AdmittedBrowserAssets {
  byId: Map<string, AdmittedBrowserAsset>;
  byDigest: Map<string, AdmittedBrowserAsset>;
}

interface StagedRenderPackage {
  fixedPackageManifestJson: string;
  timelineJson: string;
  timeline: TimelineDocument;
  resourceManifestJson: string;
  resourceManifest: ResourceManifest;
  verifiedBindingBundleJson: string;
  assets: BrowserResourceLocator[];
  admittedAssets: AdmittedBrowserAssets;
  frozenResources: FrozenBundleResources;
  engine: BrowserProductEngineWire;
  planner: ProductFramePlannerPool;
  bootstrap: ProductEngineBootstrap;
  compositor: CanvasKitExecutor;
  receipt: ProductRenderReceipt;
  audioProgram: CompiledAudioProgramWire;
}

interface FulfilledFrame {
  objects: Map<number, CanvasKitExternalObject>;
  scene3dRequests?: Map<
    string,
    { canonical: Uint8Array; topology: string; width: number; height: number }
  >;
  dispose: Array<() => void>;
  videoFrames: number;
  videoTextureFrames: number;
  resourceCache: BrowserResourceCacheFrameReport;
}

interface CompletedPreview {
  result: ProductRenderResult;
  inspection: ProductFrameInspection;
  requests: NonNullable<FulfilledFrame["scene3dRequests"]>;
  width: number;
  height: number;
  scaleX: number;
  scaleY: number;
}
interface BufferedPreview extends CompletedPreview {
  bitmap: ImageBitmap;
}

interface CachedBrowserResource {
  object: CanvasKitExternalObject;
  videoFrames: number;
  videoTextureFrames: number;
}

interface ProducedBrowserResource extends CachedBrowserResource {
  bytes: number;
  dispose: () => void;
}

export interface ProductRenderStats {
  videoFrames: number;
  gpuVideoFrames: number;
  passes: number;
  programs: number;
  physicalSurfaces: number;
  maximumLiveImages: number;
  surfaceAllocations: number;
  surfaceReuses: number;
  surfaceAllocatedBytes: number;
  surfaceResidentBytes: number;
  surfacePeakBytes: number;
  surfacePoolEvictions: number;
  templateCacheHit: boolean;
  programCacheHits: number;
  programCacheMisses: number;
  fontCacheHits: number;
  fontCacheMisses: number;
  shaderCacheHits: number;
  shaderCacheMisses: number;
  rasterLayerCacheHits: number;
  rasterLayerCacheMisses: number;
  rasterLayerCacheEntries: number;
  rasterLayerCacheBytes: number;
  resourceCacheGeneration: number;
  resourceCacheGenerationInvalidations: number;
  resourceCacheHits: number;
  resourceCacheMisses: number;
  resourceCacheInsertions: number;
  resourceCacheEvictions: number;
  resourceCacheBypasses: number;
  resourceCacheResidentEntries: number;
  resourceCacheResidentBytes: number;
}

export interface ProductHitRect {
  clipId: string;
  kind: string;
  rect: { x: number; y: number; w: number; h: number };
}

interface ProductFrameInspection {
  renderId: string;
  compositionFrame: number;
  hitRects: ProductHitRect[];
  motion: Array<{
    clipId: string;
    compositionFrame: number;
    sourceIndex: number;
    sourceFrame: number;
    boxes: Record<string, [number, number, number, number]>;
  }>;
  scene3d: Array<{
    clipId: string;
    contentHash: string;
    bounds: { x: number; y: number; width: number; height: number };
    deviceFromLocal: [
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
  }>;
}

export interface ProductRenderResult {
  implementation: typeof BROWSER_PLAYER_IMPLEMENTATION;
  runtimeFlavor: "product-gpu" | "product-cpu";
  executionProfile: CanvasKitExecutionProfile;
  fontsRegistered: number;
  png: Uint8Array | null;
  frame: {
    renderId: string;
    index: number;
    planBytes: number;
    bindingBytes: number;
  };
  stats: ProductRenderStats;
}

/**
 * Frame-inclusive Product Compositor cost attributed to an active Motion clip. During a
 * transition the same frame cost is intentionally recorded for both clips: these counters answer
 * "which active content made this frame expensive", not how to apportion shared compositor work.
 */
export interface MotionClipPerformanceStats {
  frames: number;
  evaluatePrepareMs: number;
  requestInspectMs: number;
  lowerMs: number;
  bindPacketsMs: number;
  workerMs: number;
  frameMs: number;
  executeMs: number;
  packetAdmissionMs: number;
  programAdmissionMs: number;
  scheduleAdmissionMs: number;
  renderMs: number;
  bindingBytes: number;
  scheduleBytes: number;
}

export interface PlayerStats {
  rAFFrames: number;
  renders: number;
  scrubs: number;
  fontsRegistered: number;
  videoFrames: number;
  gpuVideoFrames: number;
  clockMode: string;
  surfaceMode: string;
  gpuFrames: number;
  cpuFrames: number;
  audioMode: string;
  audioScheduledSources: number;
  audioDecodeErrors: number;
  audioPeaksDecodes: number;
  preparedFrames: number;
  perfFrameMs: number;
  perfEvaluatePrepareMs: number;
  perfRequestInspectMs: number;
  /** Fulfillment wall span; it intentionally overlaps synchronous lower work. */
  perfResourceFulfillMs: number;
  perfLowerMs: number;
  perfBindPacketsMs: number;
  perfExecuteMs: number;
  perfExecutorPacketAdmissionMs: number;
  perfExecutorProgramAdmissionMs: number;
  perfExecutorScheduleAdmissionMs: number;
  perfExecutorRenderMs: number;
  programLocalPasses: number;
  directRasterPrograms: number;
  programGroups: number;
  programTransformGroups: number;
  programClipGroups: number;
  pathClipElisions: number;
  programOpacityGroups: number;
  programFilterGroups: number;
  programMaskGroups: number;
  programShaderGroups: number;
  programBackdropGroups: number;
  programBlendGroups: number;
  perfPresentMs: number;
  perfCaptureEncodeMs: number;
  perfVideoDecodeMs: number;
  perfVideoCopyMs: number;
  perfVideoWrapMs: number;
  videoThumbExtracts: number;
  planBytes: number;
  bindingBytes: number;
  scheduleBytes: number;
  requestBytes: number;
  executionPasses: number;
  physicalSurfaces: number;
  maximumLiveImages: number;
  surfaceAllocations: number;
  surfaceReuses: number;
  surfaceAllocatedBytes: number;
  maximumSurfaceResidentBytes: number;
  maximumSurfacePeakBytes: number;
  surfacePoolEvictions: number;
  templateCacheHits: number;
  templateCacheMisses: number;
  programCacheHits: number;
  programCacheMisses: number;
  fontCacheHits: number;
  fontCacheMisses: number;
  shaderCacheHits: number;
  shaderCacheMisses: number;
  rasterLayerCacheHits: number;
  rasterLayerCacheMisses: number;
  maximumRasterLayerCacheEntries: number;
  maximumRasterLayerCacheBytes: number;
  resourceCacheGenerationInvalidations: number;
  resourceCacheHits: number;
  resourceCacheMisses: number;
  resourceCacheInsertions: number;
  resourceCacheEvictions: number;
  resourceCacheBypasses: number;
  maximumResourceCacheEntries: number;
  maximumResourceCacheBytes: number;
  renderId: string;
  framePlannerWorkers: number;
  framePlannerReadyHits: number;
  framePlannerWaitHits: number;
  framePlannerQueued: number;
  framePlannerCompleted: number;
  framePlannerCancelled: number;
  framePlannerErrors: number;
  framePlannerQueueDepth: number;
  framePlannerReadyFrames: number;
  framePlannerReadyBytes: number;
  perfFramePlannerWorkerMs: number;
  perfFramePlannerTurnGapMs: number;
  perfFramePlannerReleaseMs: number;
  motionExecuteClips: Record<string, number>;
  motionPerformance: Record<string, MotionClipPerformanceStats>;
  degradations?: Array<{ clipId?: string; effect: string; message?: string }>;
}

export const CANVASKIT_VERSION = canvasKitPackage.version;
export const BROWSER_PLAYER_IMPLEMENTATION = `valle-product-compositor+canvaskit-wasm@${CANVASKIT_VERSION}`;

let canvasKitPromise: Promise<CanvasKit> | null = null;
const staticCanvasKitInit =
  typeof globalThis.CanvasKitInit === "function"
    ? globalThis.CanvasKitInit
    : null;

export async function createBrowserValleWebPlayer(
  options: BrowserValleWebPlayerOptions,
): Promise<BrowserValleWebPlayer> {
  const player = new BrowserValleWebPlayer(options);
  try { await player.init(); return player; }
  catch (error) { await player.close(); throw error; }
}

export function resolveAssetUrl(
  value: string,
  assetBaseUrl = "/assets/",
  proxyBase: string | null = null,
): string {
  if (/^https?:\/\//.test(value) && proxyBase) {
    return `${proxyBase}?url=${encodeURIComponent(value)}`;
  }
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(value) || value.startsWith("/"))
    return value;
  const base = assetBaseUrl.endsWith("/") ? assetBaseUrl : `${assetBaseUrl}/`;
  return `${base}${value.split("/").map(encodeURIComponent).join("/")}`;
}

export class BrowserValleWebPlayer {
  fixedPackageManifestJson: string;
  timelineJson: string;
  timeline: TimelineDocument;
  resourceManifestJson: string;
  resourceManifest: ResourceManifest;
  verifiedBindingBundleJson: string;
  assets: BrowserResourceLocator[];
  canvas: HTMLCanvasElement | null;
  assetBaseUrl: string;
  proxyBase: string | null;
  pxScale: number;
  gpu: boolean;
  playing = false;
  muted = false;
  closed = false;
  onTimeUpdate?: (timeS: number) => void;
  /** Optional presentation diagnostics. Never participates in frame evaluation or scheduling. */
  onFramePresented?: (frame: number, atMs: number, clockTimeS: number) => void;
  readonly stats: PlayerStats;
  hitRects: ProductHitRect[] = [];

  private readonly trace?: RenderTrace;
  private readonly loadMotionFonts?: () => Promise<string>;
  private readonly runtimeAssets: PlayerRuntimeAssets;
  private readonly wasmModuleUrl: string;
  private readonly wasmUrl: string;
  private wasm!: ValleWasmModule;
  private engine!: BrowserProductEngineWire;
  private planner: ProductFramePlannerPool | null = null;
  private scene3dDof: CanvasKitScene3dDof | null = null;
  private scene3dGpu: GpuScene3D | null = null;
  private scene3dGpuInit: Promise<GpuScene3D | null> | null = null;
  private gpuCircles: GpuCircles | null = null;
  private currentScene3dRequests = new Map<
    string,
    { canonical: Uint8Array; topology: string; width: number; height: number }
  >();
  private readonly renderMode: "reference" | "realtime";
  readonly runtimeBudget: BrowserRuntimeBudget;
  private readonly runtimePurpose: "preview" | "export";
  private readonly runtimeAbort = new AbortController();
  private readonly memoryOwner = {};
  private bootstrap!: ProductEngineBootstrap;
  private CanvasKit!: CanvasKit;
  private compositor!: CanvasKitExecutor;
  private renderId = "";
  private generation = 1n;
  private timeS = 0;
  private playGeneration = 0;
  private rafId: number | null = null;
  private renderInFlight: Promise<unknown> | null = null;
  private renderTail: Promise<void> | null = null;
  private warmedPreviewRenderId: string | null = null;
  private warmedPreviewTarget: TargetSurface | null = null;
  private playbackPump: Promise<void> | null = null;
  private previewBuffer: PreviewBuffer<BufferedPreview> | null = null;
  private audioPump: Promise<void> | null = null;
  private playbackFrame: number | null = null;
  private target: TargetSurface | null = null;
  private readonly assetsById = new Map<string, AdmittedBrowserAsset>();
  private readonly assetByDigest = new Map<string, AdmittedBrowserAsset>();
  private readonly assetBytes = new Map<string, Promise<Uint8Array>>();
  private readonly videoBlobs = new MediaBlobCache();
  private readonly scene3dResourceTasks = new Map<string, Promise<void>>();
  private readonly videoRings = new Map<string, DecoderRing>();
  private readonly videoRingUsers = new Map<string, number>();
  private readonly fontBytes = new Map<string, Uint8Array>();
  private readonly modelBytes = new Map<string, Uint8Array>();
  private readonly environmentBytes = new Map<string, Uint8Array>();
  private readonly shaderBytes = new Map<string, Uint8Array>();
  private readonly resourceObjects: BrowserResourceCache<CachedBrowserResource>;
  private renderReceipt: ProductRenderReceipt | null = null;
  private compiledAudioProgram: CompiledAudioProgramWire | null = null;
  private audioBuffers = new Map<string, AudioBuffer>();
  private audioReaders = new Map<string, BrowserAudioWindowReader>();
  private audioTail: Promise<unknown> = Promise.resolve();
  private readonly audioPeakTasks = new Set<Promise<unknown>>();
  private readonly audioMemoryOwner = {};
  private audioGraph: AudioGraph | null = null;
  private readonly audioContext: AudioContext | null;
  private readonly clock: AudioMasterClock;
  private volume = 1;
  private frameInspection: ProductFrameInspection = {
    renderId: "",
    compositionFrame: -1,
    hitRects: [],
    motion: [],
    scene3d: [],
  };
  private inspectedFrame: number | null = null;
  private inspectionScaleX = 1;
  private inspectionScaleY = 1;
  private lastRender: ProductRenderResult | null = null;

  constructor(options: BrowserValleWebPlayerOptions) {
    if (!options?.fixedPackageManifestJson)
      throw new Error("fixedPackageManifestJson is required");
    if (!options?.timelineJson) throw new Error("timelineJson is required");
    if (!options.resourceManifestJson)
      throw new Error("resourceManifestJson is required");
    if (!options.verifiedBindingBundleJson)
      throw new Error("verifiedBindingBundleJson is required");
    if (options.cpuThreads !== undefined && (!Number.isSafeInteger(options.cpuThreads) || options.cpuThreads < 1 || options.cpuThreads > 8)) {
      throw new RangeError("cpuThreads must be an integer in 1..8");
    }
    this.runtimeBudget = options.runtimeBudget ?? sharedBrowserRuntimeBudget(options.cpuThreads, options.memoryBytes);
    this.runtimePurpose = options.runtimePurpose ?? "preview";
    this.renderMode = options.renderMode ?? "reference";
    if (this.renderMode !== "reference" && this.renderMode !== "realtime")
      throw new Error("Invalid renderMode");
    this.runtimeAssets = resolvePlayerRuntimeAssets(
      options.runtimeAssets,
      options.runtimeBaseUrl,
    );
    this.wasmModuleUrl = this.runtimeAssets.engine.glue;
    this.wasmUrl = this.runtimeAssets.engine.wasm;
    this.fixedPackageManifestJson = options.fixedPackageManifestJson;
    this.timelineJson = options.timelineJson;
    this.timeline = JSON.parse(options.timelineJson) as TimelineDocument;
    this.resourceManifestJson = options.resourceManifestJson;
    this.resourceManifest = JSON.parse(
      options.resourceManifestJson,
    ) as ResourceManifest;
    this.verifiedBindingBundleJson = options.verifiedBindingBundleJson;
    this.assets = options.assets ?? [];
    if (options.fonts !== undefined) {
      const pageUrl =
        typeof location === "undefined" ? import.meta.url : location.href;
      this.loadMotionFonts = createMotionFontLoader(
        options.fonts,
        new URL(options.assetBaseUrl ?? pageUrl, pageUrl).href,
      );
    }
    this.canvas = options.canvas ?? null;
    this.assetBaseUrl = options.assetBaseUrl ?? "/assets/";
    this.proxyBase = options.proxyBase ?? null;
    this.pxScale = finitePositive(options.pxScale ?? 1, "pxScale");
    this.gpu = options.gpu ?? true;
    this.trace = options.trace;
    this.audioContext = options.audioContext === undefined
      ? maybeCreateAudioContext(this.timeline.document.canvas.sampleRate)
      : options.audioContext;
    this.resourceObjects = new BrowserResourceCache(options.resourceCache);
    this.clock = new AudioMasterClock(this.audioContext);
    this.stats = emptyPlayerStats();
    this.runtimeBudget.register(this.audioMemoryOwner, () => this.audioBuffers.clear());
    this.runtimeBudget.register(this.memoryOwner, () => {
      this.resourceObjects.dispose();
      this.compositor?.trimWorkingMemory();
    });
  }

  async init(): Promise<this> {
    [this.wasm, this.CanvasKit] = await Promise.all([
      loadWasmModule(this.wasmModuleUrl, this.wasmUrl),
      loadCanvasKit(this.runtimeAssets.canvasKit),
    ]).then(([wasm, canvasKit]) => [wasm, canvasKit]);
    const stageStarted = performance.now();
    const staged = await this.stageRenderPackage(this.currentRenderPackage());
    this.commitStagedRenderPackage(staged);
    performance.mark("valle-player-package-stage", {
      detail: {
        mode: "initial",
        durationMs: performance.now() - stageStarted,
      },
    });
    return this;
  }

  durationS(): number {
    const receipt = this.requireRenderReceipt();
    return receipt.sampleCount / receipt.sampleRate;
  }

  /** The admitted frame/sample schedule, independent of preview resolution and playback clocks. */
  get renderInfo(): Readonly<ProductRenderReceipt> {
    return { ...this.requireRenderReceipt() };
  }

  get scene3dGpuDiagnostics() {
    return this.scene3dGpu ? { ...this.scene3dGpu.stats } : null;
  }
  get geometryGpuDiagnostics() {
    return this.gpuCircles ? { ...this.gpuCircles.stats } : null;
  }

  lastFrameTimeS(): number {
    const receipt = this.requireRenderReceipt();
    return (
      (receipt.frameCount - 1) /
      canonicalRationalNumber(receipt.frameRate, "frameRate")
    );
  }

  currentTime(): number {
    return this.timeS;
  }

  async seek(timeS: number, options: RenderOptions = {}) {
    this.pause();
    const previous = this.renderInFlight;
    // Reserve the render slot before yielding so rapid seeks cannot invalidate each other's
    // demanded planning tasks. Each caller still receives the frame it requested.
    const job = (async () => {
      if (previous) await previous.catch(() => undefined);
      this.requirePlanner().advanceEpoch();
      this.timeS = clamp(timeS, 0, this.lastFrameTimeS());
      const rendered = await this.renderTime(this.timeS, options);
      this.playbackFrame = rendered.frame.index;
      this.onTimeUpdate?.(this.timeS);
      return rendered;
    })();
    this.renderInFlight = job;
    try {
      return await job;
    } finally {
      if (this.renderInFlight === job) this.renderInFlight = null;
    }
  }

  async scrub(timeS: number) {
    this.stats.scrubs += 1;
    return this.seek(timeS, { png: true });
  }

  async renderTime(timeS: number, options: RenderOptions = {}) {
    const frame =
      options.frame ??
      this.frameAtSeconds(clamp(timeS, 0, this.lastFrameTimeS()));
    return this.renderFrame(frame, options);
  }

  async renderFrame(frame: number, options: RenderOptions = {}) {
    return (await this.renderFrameDetailed(frame, options, true)).result;
  }
  private renderFrameDetailed(
    frame: number,
    options: RenderOptions,
    publish: boolean,
    afterRender?: (completed: CompletedPreview) => Promise<void>,
  ): Promise<CompletedPreview> {
    // Working surfaces and GPU fulfillment belong to one frame at a time, including when
    // cooperative drawing yields to presentation, a seek or a direct SDK render call.
    const job = (this.renderTail ?? Promise.resolve()).then(async () => {
      const priority: RuntimePriority = this.runtimePurpose === "export" ? "export" : this.playing ? "playback" : "interaction";
      const release = await this.runtimeBudget.acquirePresentation(priority, this.runtimeAbort.signal);
      let completed: CompletedPreview;
      try { completed = await this.renderFrameNow(frame, options, publish); }
      finally { release(); }
      // Deferred presentation still owns this GPU target until its deadline. SDK calls and seeks
      // must not overwrite it between drawing a frame and presenting that frame.
      await afterRender?.(completed);
      return completed;
    });
    this.renderTail = job.then(() => undefined, () => undefined);
    return job;
  }
  private async renderFrameNow(
    frame: number,
    options: RenderOptions,
    publish: boolean,
    cooperative = true,
  ): Promise<CompletedPreview> {
    if (this.closed) throw new Error("browser player is closed");
    const renderId = this.renderId;
    if (!renderId) throw new Error("browser player has no open RenderId");
    const receipt = this.requireRenderReceipt();
    const frameCount = receipt.frameCount;
    if (!Number.isSafeInteger(frame) || frame < 0 || frame >= frameCount) {
      throw new RangeError(
        `frame ${frame} is outside fixed render range [0, ${frameCount})`,
      );
    }
    const canvasWidth = receipt.canvasWidth;
    const canvasHeight = receipt.canvasHeight;
    const scale = finitePositive(
      options.rasterScale ?? this.pxScale,
      "rasterScale",
    );
    const width = Math.max(1, Math.round(canvasWidth * scale));
    const height = Math.max(1, Math.round(canvasHeight * scale));
    const forceCpu = options.png === true;
    const frameStarted = performance.now();
    const planningInput = {
      ...this.framePlanningInput(
        frame,
        width,
        height,
        options.transparent === true,
      ),
      trace: this.trace !== undefined,
    };
    const planner = this.requirePlanner();
    const endPrepareWait = this.trace?.span("prepare.wait", { frame });
    const planning = planner.prepare(planningInput);
    let fulfilled: FulfilledFrame | null = null;
    let finishMemory: ((retainedBytes: number) => void) | undefined;
    let retainedBytes = 0;
    try {
      const requestStage = await planning.requests;
      endPrepareWait?.();
      this.trace?.worker(
        "prepare.evaluate",
        requestStage.workerTimeOriginMs,
        requestStage.evaluateStartedMs,
        requestStage.evaluatePrepareMs,
        frame,
      );
      this.trace?.worker(
        "prepare.inspect",
        requestStage.workerTimeOriginMs,
        requestStage.inspectStartedMs,
        requestStage.requestInspectMs,
        frame,
      );
      this.assertActiveRenderId(renderId, requestStage.renderId);
      const frameInspection = JSON.parse(
        requestStage.inspectionJson,
      ) as ProductFrameInspection;
      this.assertActiveRenderId(renderId, frameInspection.renderId);
      if (frameInspection.compositionFrame !== frame) {
        throw new Error(
          `Product inspection frame drift: expected ${frame}, got ${frameInspection.compositionFrame}`,
        );
      }
      for (const motion of frameInspection.motion) {
        if (motion.compositionFrame !== frame) {
          throw new Error(
            `Motion inspection frame drift for '${motion.clipId}': expected ${frame}, got ${motion.compositionFrame}`,
          );
        }
      }
      this.stats.requestBytes += requestStage.requestPacket.byteLength;
      const readyStage = await planning.ready;
      const schedule = record(await decodePackedAbi(readyStage.schedulePacket, BOUND_PROGRAM_SCHEDULES_ABI), "working-set schedule");
      const requests = await decodePackedAbi(requestStage.requestPacket, RESOURCE_REQUESTS_ABI);
      if (!Array.isArray(requests)) throw new Error("ResourceRequestSet must decode to an array");
      const programBytes = Math.max(0, ...(schedule.programs as Wire[]).map(program => Number(program.estimatedSurfaceBytes)));
      const outer = (schedule.surfaceSlots as Wire[]).reduce((sum, slot) => sum + Number(slot.estimatedBytes), 0);
      let required = outer + programBytes * 5 + width * height * 32 + 8 * 1024 ** 2;
      for (const request of requests as Wire[]) {
        const expected = request.expected;
        if (expected.extent) required += Number(expected.extent.width) * Number(expected.extent.height) * 32;
        if (expected.kind === "scene3d") {
          const value = JSON.parse(new TextDecoder().decode(packedByteArray(request.payload.canonical_request, "Scene3D request")));
          required += value.width * value.height * 128;
        }
      }
      // The frame envelope now owns the projected packets, replacing their background leases.
      planner.adoptPreparationMemory(planning.key);
      finishMemory = await this.runtimeBudget.admitFrameAfterKernels(this.memoryOwner, required, this.runtimeAbort.signal);
      const target = this.acquireTarget(width, height, forceCpu);
      const fulfillmentStarted = performance.now();
      const endFulfill = this.trace?.span("resource.fulfill", { frame });
      fulfilled = await this.fulfillRequests(requestStage.requestPacket, target.gpu, requestStage.gpuScene3d)
        .finally(() => endFulfill?.());
      this.trace?.worker(
        "lower",
        readyStage.workerTimeOriginMs,
        readyStage.lowerStartedMs,
        readyStage.lowerMs,
        frame,
      );
      this.trace?.worker(
        "bind",
        readyStage.workerTimeOriginMs,
        readyStage.bindStartedMs,
        readyStage.bindPacketsMs,
        frame,
      );
      this.assertActiveRenderId(renderId, readyStage.renderId);
      const frameResources = fulfilled;
      const resourceFulfillMs = performance.now() - fulfillmentStarted;
      const generation = readyStage.generation;
      const planPacket = readyStage.planPacket;
      const bindingPacket = readyStage.bindingPacket;
      const schedulePacket = readyStage.schedulePacket;
      this.stats.planBytes += readyStage.planPacketTransferred
        ? planPacket.byteLength
        : 0;
      this.stats.bindingBytes += bindingPacket.byteLength;
      this.stats.scheduleBytes += schedulePacket.byteLength;
      // GPU fulfillment and first-use material admission can each consume most of a frame
      // interval. Give presentation its turn before combining both into one long main task.
      if (!publish && target.gpu && cooperative && performance.now() - frameStarted >= 6)
        await yieldToHost();
      const endExecute = this.trace?.span("execute.record", {
        frame,
        gpu: target.gpu,
      });
      const executeStarted = performance.now();
      const passTimings = this.trace
        ? new Map<
            string,
            {
              count: number;
              pixels: number;
              drawMs: number;
              snapshotMs: number;
            }
          >()
        : null;
      this.assertActiveRenderId(renderId, readyStage.renderId);
      const report = await this.compositor.execute(
        planPacket,
        bindingPacket,
        schedulePacket,
        { generation, objects: frameResources.objects },
        {
          surface: target.surface,
          deferGpuFlush: this.renderMode === "realtime",
          boundedPathClipLayers: this.renderMode === "realtime",
          fuseClipPaints: this.renderMode === "realtime",
          retainRasterTrees: this.renderMode === "realtime",
          geometryRenderer: this.gpuCircles,
          yieldToPresentation: !publish && target.gpu && cooperative
            ? yieldToHost
            : undefined,
          directContext: target.context,
          maxSurfaceBytes: planningInput.maxSurfaceBytes,
          maxFrameBytes: planningInput.maxFrameBytes,
          onPassTiming: passTimings
            ? (timing) => {
                const totals = passTimings.get(timing.kind) ?? {
                  count: 0,
                  pixels: 0,
                  drawMs: 0,
                  snapshotMs: 0,
                };
                totals.count++;
                totals.pixels += timing.width * timing.height;
                totals.drawMs += timing.drawMs;
                totals.snapshotMs += timing.snapshotMs;
                passTimings.set(timing.kind, totals);
              }
            : undefined,
        },
      );
      this.assertActiveRenderId(renderId, readyStage.renderId);
      retainedBytes = Number(report.surfaceResidentBytes) + report.rasterLayerCacheBytes + frameResources.resourceCache.residentBytes + planner.snapshot().readyBytes;
      const executeMs = performance.now() - executeStarted;
      endExecute?.();
      if (passTimings)
        this.trace?.evidence("execute.breakdown", {
          frame,
          packetAdmissionMs: report.packetAdmissionMs,
          programAdmissionMs: report.programAdmissionMs,
          scheduleAdmissionMs: report.scheduleAdmissionMs,
          renderMs: report.renderMs,
          passes: Object.fromEntries(passTimings),
          directRasterPrograms: report.directRasterPrograms,
        });
      const endPresent = this.trace?.span("delivery.present", {
        frame,
        cpuReadback: !target.gpu,
      });
      const presentStarted = performance.now();
      if (publish) this.presentTarget(target, width, height);
      const presentMs = performance.now() - presentStarted;
      endPresent?.();
      const captureEncodeStarted = performance.now();
      const png = options.png ? this.encodeTargetPng(target.surface) : null;
      const captureEncodeMs = performance.now() - captureEncodeStarted;
      const stats: ProductRenderStats = {
        videoFrames: frameResources.videoFrames,
        gpuVideoFrames: frameResources.videoTextureFrames,
        passes: report.passes,
        programs: report.programs,
        physicalSurfaces: report.physicalSurfaces,
        maximumLiveImages: report.maximumLiveImages,
        surfaceAllocations: report.surfaceAllocations,
        surfaceReuses: report.surfaceReuses,
        surfaceAllocatedBytes: exactBigIntNumber(
          report.surfaceAllocatedBytes,
          "surface allocated bytes",
        ),
        surfaceResidentBytes: exactBigIntNumber(
          report.surfaceResidentBytes,
          "surface resident bytes",
        ),
        surfacePeakBytes: exactBigIntNumber(
          report.surfacePeakBytes,
          "surface peak bytes",
        ),
        surfacePoolEvictions: report.surfacePoolEvictions,
        templateCacheHit: readyStage.templateCacheHit,
        programCacheHits: report.programCacheHits,
        programCacheMisses: report.programCacheMisses,
        fontCacheHits: report.fontCacheHits,
        fontCacheMisses: report.fontCacheMisses,
        shaderCacheHits: report.shaderCacheHits,
        shaderCacheMisses: report.shaderCacheMisses,
        rasterLayerCacheHits: report.rasterLayerCacheHits,
        rasterLayerCacheMisses: report.rasterLayerCacheMisses,
        rasterLayerCacheEntries: report.rasterLayerCacheEntries,
        rasterLayerCacheBytes: report.rasterLayerCacheBytes,
        resourceCacheGeneration: frameResources.resourceCache.generation,
        resourceCacheGenerationInvalidations:
          frameResources.resourceCache.generationInvalidations,
        resourceCacheHits: frameResources.resourceCache.hits,
        resourceCacheMisses: frameResources.resourceCache.misses,
        resourceCacheInsertions: frameResources.resourceCache.insertions,
        resourceCacheEvictions: frameResources.resourceCache.evictions,
        resourceCacheBypasses: frameResources.resourceCache.bypasses,
        resourceCacheResidentEntries:
          frameResources.resourceCache.residentEntries,
        resourceCacheResidentBytes: frameResources.resourceCache.residentBytes,
      };
      this.trace?.evidence("frame.caches", {
        frame,
        ...stats,
        requestBytes: requestStage.requestPacket.byteLength,
        planBytes: readyStage.planPacketTransferred ? planPacket.byteLength : 0,
        bindingBytes: bindingPacket.byteLength,
        scheduleBytes: schedulePacket.byteLength,
      });
      const frameMs = performance.now() - frameStarted;
      this.stats.renders += 1;
      this.stats.preparedFrames += 1;
      this.stats.perfFrameMs += frameMs;
      this.stats.perfEvaluatePrepareMs += requestStage.evaluatePrepareMs;
      this.stats.perfRequestInspectMs += requestStage.requestInspectMs;
      this.stats.perfResourceFulfillMs += resourceFulfillMs;
      this.stats.perfLowerMs += readyStage.lowerMs;
      this.stats.perfBindPacketsMs += readyStage.bindPacketsMs;
      this.stats.perfFramePlannerWorkerMs += readyStage.totalWorkerMs;
      this.stats.perfFramePlannerTurnGapMs += requestStage.workerTurnGapMs;
      this.stats.perfFramePlannerReleaseMs += requestStage.previousReleaseMs;
      this.stats.perfExecuteMs += executeMs;
      this.stats.perfExecutorPacketAdmissionMs += report.packetAdmissionMs;
      this.stats.perfExecutorProgramAdmissionMs += report.programAdmissionMs;
      this.stats.perfExecutorScheduleAdmissionMs += report.scheduleAdmissionMs;
      this.stats.perfExecutorRenderMs += report.renderMs;
      this.stats.programLocalPasses += report.programLocalPasses;
      this.stats.directRasterPrograms += report.directRasterPrograms;
      this.stats.programGroups += report.programGroups;
      this.stats.programTransformGroups += report.programTransformGroups;
      this.stats.programClipGroups += report.programClipGroups;
      this.stats.pathClipElisions += report.pathClipElisions;
      this.stats.programOpacityGroups += report.programOpacityGroups;
      this.stats.programFilterGroups += report.programFilterGroups;
      this.stats.programMaskGroups += report.programMaskGroups;
      this.stats.programShaderGroups += report.programShaderGroups;
      this.stats.programBackdropGroups += report.programBackdropGroups;
      this.stats.programBlendGroups += report.programBlendGroups;
      this.stats.perfPresentMs += presentMs;
      this.stats.perfCaptureEncodeMs += captureEncodeMs;
      this.stats.videoFrames += stats.videoFrames;
      this.stats.gpuVideoFrames += stats.gpuVideoFrames;
      this.stats.executionPasses += report.passes;
      this.stats.physicalSurfaces = Math.max(
        this.stats.physicalSurfaces,
        report.physicalSurfaces,
      );
      this.stats.maximumLiveImages = Math.max(
        this.stats.maximumLiveImages,
        report.maximumLiveImages,
      );
      this.stats.surfaceAllocations += report.surfaceAllocations;
      this.stats.surfaceReuses += report.surfaceReuses;
      this.stats.surfaceAllocatedBytes += exactBigIntNumber(
        report.surfaceAllocatedBytes,
        "surface allocated bytes",
      );
      this.stats.maximumSurfaceResidentBytes = Math.max(
        this.stats.maximumSurfaceResidentBytes,
        exactBigIntNumber(
          report.surfaceResidentBytes,
          "surface resident bytes",
        ),
      );
      this.stats.maximumSurfacePeakBytes = Math.max(
        this.stats.maximumSurfacePeakBytes,
        exactBigIntNumber(report.surfacePeakBytes, "surface peak bytes"),
      );
      this.stats.surfacePoolEvictions += report.surfacePoolEvictions;
      if (readyStage.templateCacheHit) this.stats.templateCacheHits += 1;
      else this.stats.templateCacheMisses += 1;
      this.stats.programCacheHits += report.programCacheHits;
      this.stats.programCacheMisses += report.programCacheMisses;
      this.stats.fontCacheHits += report.fontCacheHits;
      this.stats.fontCacheMisses += report.fontCacheMisses;
      this.stats.shaderCacheHits += report.shaderCacheHits;
      this.stats.shaderCacheMisses += report.shaderCacheMisses;
      this.stats.rasterLayerCacheHits += report.rasterLayerCacheHits;
      this.stats.rasterLayerCacheMisses += report.rasterLayerCacheMisses;
      this.stats.maximumRasterLayerCacheEntries = Math.max(
        this.stats.maximumRasterLayerCacheEntries,
        report.rasterLayerCacheEntries,
      );
      this.stats.maximumRasterLayerCacheBytes = Math.max(
        this.stats.maximumRasterLayerCacheBytes,
        report.rasterLayerCacheBytes,
      );
      this.stats.resourceCacheGenerationInvalidations +=
        frameResources.resourceCache.generationInvalidations;
      this.stats.resourceCacheHits += frameResources.resourceCache.hits;
      this.stats.resourceCacheMisses += frameResources.resourceCache.misses;
      this.stats.resourceCacheInsertions +=
        frameResources.resourceCache.insertions;
      this.stats.resourceCacheEvictions +=
        frameResources.resourceCache.evictions;
      this.stats.resourceCacheBypasses += frameResources.resourceCache.bypasses;
      this.stats.maximumResourceCacheEntries = Math.max(
        this.stats.maximumResourceCacheEntries,
        frameResources.resourceCache.residentEntries,
      );
      this.stats.maximumResourceCacheBytes = Math.max(
        this.stats.maximumResourceCacheBytes,
        frameResources.resourceCache.residentBytes,
      );
      if (target.gpu) this.stats.gpuFrames += 1;
      else this.stats.cpuFrames += 1;
      this.stats.surfaceMode = target.gpu ? "gpu-product" : "cpu-product";
      for (const motion of frameInspection.motion) {
        this.stats.motionExecuteClips[motion.clipId] =
          (this.stats.motionExecuteClips[motion.clipId] ?? 0) + 1;
        const clip = (this.stats.motionPerformance[motion.clipId] ??= {
          frames: 0,
          evaluatePrepareMs: 0,
          requestInspectMs: 0,
          lowerMs: 0,
          bindPacketsMs: 0,
          workerMs: 0,
          frameMs: 0,
          executeMs: 0,
          packetAdmissionMs: 0,
          programAdmissionMs: 0,
          scheduleAdmissionMs: 0,
          renderMs: 0,
          bindingBytes: 0,
          scheduleBytes: 0,
        });
        clip.frames += 1;
        clip.evaluatePrepareMs += requestStage.evaluatePrepareMs;
        clip.requestInspectMs += requestStage.requestInspectMs;
        clip.lowerMs += readyStage.lowerMs;
        clip.bindPacketsMs += readyStage.bindPacketsMs;
        clip.workerMs += readyStage.totalWorkerMs;
        clip.frameMs += frameMs;
        clip.executeMs += executeMs;
        clip.packetAdmissionMs += report.packetAdmissionMs;
        clip.programAdmissionMs += report.programAdmissionMs;
        clip.scheduleAdmissionMs += report.scheduleAdmissionMs;
        clip.renderMs += report.renderMs;
        clip.bindingBytes += bindingPacket.byteLength;
        clip.scheduleBytes += schedulePacket.byteLength;
      }
      this.syncPlannerStats();
      const result: ProductRenderResult = {
        implementation: BROWSER_PLAYER_IMPLEMENTATION,
        runtimeFlavor: target.gpu ? "product-gpu" : "product-cpu",
        executionProfile: report.profile,
        fontsRegistered: this.stats.fontsRegistered,
        png,
        frame: {
          renderId,
          index: frame,
          planBytes: planPacket.byteLength,
          bindingBytes: bindingPacket.byteLength,
        },
        stats,
      };
      const completed = {
        result,
        inspection: frameInspection,
        requests: frameResources.scene3dRequests ?? new Map(),
        width,
        height,
        scaleX: width / canvasWidth,
        scaleY: height / canvasHeight,
      };
      if (publish) this.publishInspection(completed);
      return completed;
    } finally {
      try {
        planner.release(planning.key);
        if (fulfilled) {
          try { this.resourceObjects.releaseFrame(); }
          finally { disposeAll(fulfilled.dispose); }
        }
      } finally {
        if (finishMemory && retainedBytes === 0) {
          this.resourceObjects.dispose();
          this.compositor?.trimWorkingMemory();
        }
        finishMemory?.(retainedBytes);
      }
    }
  }

  private publishInspection(completed: CompletedPreview): void {
    this.lastRender = completed.result;
    this.frameInspection = completed.inspection;
    this.currentScene3dRequests = completed.requests;
    this.hitRects = completed.inspection.hitRects;
    this.inspectedFrame = completed.result.frame.index;
    this.inspectionScaleX = completed.scaleX;
    this.inspectionScaleY = completed.scaleY;
    this.onFramePresented?.(
      completed.result.frame.index,
      performance.now(),
      this.playing
        ? clamp(this.clock.now(), 0, this.lastFrameTimeS())
        : this.timeS,
    );
  }

  async play(): Promise<void> {
    if (this.closed) throw new Error("browser player is closed");
    if (this.playing) return;
    const generation = ++this.playGeneration;
    if (this.renderInFlight) await this.renderInFlight.catch(() => undefined);
    if (this.renderTail) await this.renderTail;
    if (generation !== this.playGeneration || this.closed) return;
    if (this.timeS >= this.lastFrameTimeS()) this.timeS = 0;
    if (this.renderMode === "realtime") {
      const receipt = this.requireRenderReceipt();
      this.acquireTarget(Math.max(1, Math.round(receipt.canvasWidth * this.pxScale)),
        Math.max(1, Math.round(receipt.canvasHeight * this.pxScale)), false);
    }
    // Distant video frames require decoder seeks; keep their preview warmup sequential.
    const hasVideoInputs = this.renderMode === "realtime" &&
      [...this.assetByDigest.values()].some(asset => asset.type === "video");
    if (this.renderMode === "realtime" && !hasVideoInputs && this.target?.gpu &&
        (this.warmedPreviewRenderId !== this.renderId || this.warmedPreviewTarget !== this.target)) {
      const renderId = this.renderId, last = this.requireRenderReceipt().frameCount - 1;
      // Compile common material/coverage variants before starting the clock. Sample the work's
      // time range rather than special-casing a film, and retain no rendered sample bitmaps.
      // Neighboring frames can switch native coverage/material variants at an extent or alpha
      // boundary. Warm both sides of each sampled boundary instead of discovering the second
      // pipeline synchronously after the playback clock has started.
      const samples = new Set([Math.floor(last / 4), Math.floor(last / 2), Math.floor(last * 3 / 4), last]
        .flatMap(frame => [frame, Math.min(last, frame + 1)]));
      for (const frame of samples) {
        if (generation !== this.playGeneration || this.closed || this.renderId !== renderId) return;
        const pending = this.renderFrameDetailed(frame, {}, false);
        this.renderInFlight = pending;
        try { await pending; }
        catch (error) {
          if (generation !== this.playGeneration || this.closed || this.renderId !== renderId) return;
          throw error;
        }
        finally { if (this.renderInFlight === pending) this.renderInFlight = null; }
      }
      this.warmedPreviewRenderId = renderId;
      this.warmedPreviewTarget = this.target;
    }
    const planning = this.prefetchPlanningWindow(
      this.frameAtSeconds(this.timeS), true,
    );
    const audio = this.prefetchAudio(this.timeS, this.audioContext);
    // Start the media clock only after a bounded decode-independent cushion exists. Without this
    // phase the clock spends its first transition consuming the very frames the worker is still
    // producing, and a realtime-capable planner can never recover that initial debt.
    // Await one demanded frame: speculative preparation may stop at its byte budget.
    let prefetched: CompiledAudioPlayback | null;
    try {
      [, prefetched] = await Promise.all([
        Promise.all(planning.slice(0,1).map(job=>job.ready)),
        audio,
      ]);
    } catch (error) {
      // A pause/seek can retire this play request while its initial frames are still planning.
      if (generation !== this.playGeneration || this.closed) return;
      throw error;
    }
    if (generation !== this.playGeneration || this.closed) return;
    if (!hasVideoInputs && this.renderMode === "realtime" && this.target?.gpu) {
      // Pure Motion has no pixel queue. Let parallel preparation fill a small packet cushion
      // before its media clock starts. The deadline also covers plans that cannot fit together
      // in the existing byte budget; they remain speculative and never bypass that budget.
      await Promise.race([
        Promise.allSettled(planning.slice(0, 24).map(job => job.ready)),
        waitMilliseconds(800),
      ]);
      if (generation !== this.playGeneration || this.closed) return;
    }
    if (
      this.renderMode === "realtime" &&
      hasVideoInputs &&
      this.target?.gpu &&
      this.target.canvas &&
      this.canvas &&
      typeof createImageBitmap === "function"
    ) {
      const start = this.frameAtSeconds(this.timeS),
        first = Math.max(
          start,
          this.playbackFrame === null ? start : this.playbackFrame + 1,
        );
      const target = this.target,
        capacity = Math.max(
          1,
          Math.min(
            8,
            Math.floor(
              (64 * 1024 * 1024) /
                (target.canvas!.width * target.canvas!.height * 4),
            ),
          ),
        );
      const buffer = new PreviewBuffer(
        first,
        this.requireRenderReceipt().frameCount - 1,
        capacity,
        async (frame) => {
          this.prefetchPlanningWindow(frame);
          const pending = (async () => {
            const started = performance.now();
            let preview: BufferedPreview | null = null;
            await this.renderFrameDetailed(frame, {}, false, async completed => {
              if (frame === first + capacity - 1) {
                const gl = target.canvas!.getContext("webgl2");
                if (gl) await waitForGpuCompletion(gl);
              }
              if (performance.now() - started >= 6) await yieldToHost();
              // Snapshot completion belongs to the same render slot as its GPU writes.
              const bitmap = await createImageBitmap(target.canvas!);
              preview = { ...completed, bitmap };
            });
            return preview!;
          })();
          this.renderInFlight = pending;
          try {
            return await pending;
          } finally {
            if (this.renderInFlight === pending) this.renderInFlight = null;
          }
        },
        (completed) => completed.bitmap.close(),
        () => this.frameAtSeconds(this.playing
          ? clamp(this.clock.now(), 0, this.lastFrameTimeS())
          : this.timeS),
      );
      this.previewBuffer = buffer;
      try {
        await buffer.prebuffer(capacity);
      } catch (error) {
        buffer.close();
        if (generation === this.playGeneration && !this.closed) throw error;
        return;
      }
      if (generation !== this.playGeneration || this.closed) {
        buffer.close();
        return;
      }
    }
    await this.clock.start(this.timeS, { useAudioClock: prefetched !== null });
    if (generation !== this.playGeneration || this.closed) return;
    this.teardownAudioGraph();
    if (this.audioContext && prefetched) {
      this.audioGraph = this.scheduleAudioGraph(
        this.audioContext,
        prefetched,
        this.timeS,
      );
      this.stats.audioMode = "webaudio";
    } else {
      this.stats.audioMode = "performance";
    }
    this.playing = true;
    this.stats.clockMode = this.clock.mode;
    this.scheduleTick();
    if (this.audioGraph && this.audioContext) {
      const audioPump = this.runAudioPump(
        generation,
        this.audioContext,
        this.audioGraph,
      );
      this.audioPump = audioPump;
      audioPump
        .catch((error) => {
          if (generation !== this.playGeneration || this.closed) return;
          this.pause();
          queueMicrotask(() => {
            throw error;
          });
        })
        .finally(() => {
          if (this.audioPump === audioPump) this.audioPump = null;
        });
    }
    const pump = this.runPlaybackPump(generation);
    this.playbackPump = pump;
    pump
      .catch((error) => {
        if (generation !== this.playGeneration || this.closed) return;
        this.pause();
        queueMicrotask(() => {
          throw error;
        });
      })
      .finally(() => {
        if (this.playbackPump === pump) this.playbackPump = null;
      });
  }

  pause(): void {
    this.previewBuffer?.close();
    this.previewBuffer = null;
    this.playGeneration += 1;
    if (this.rafId !== null) cancelAnimationFrame(this.rafId);
    this.rafId = null;
    if (this.playing)
      this.timeS = clamp(this.clock.now(), 0, this.lastFrameTimeS());
    this.playing = false;
    this.clock.pause(this.timeS);
    this.teardownAudioGraph();
  }

  hasAudio(): boolean {
    return (this.compiledAudioProgram?.tracks.length ?? 0) > 0;
  }

  setVolume(value: number): number {
    this.volume = clamp(value, 0, 1);
    this.applyMasterVolume();
    return this.volume;
  }

  get masterVolume(): number {
    return this.volume;
  }

  setMuted(flag: boolean): boolean {
    this.muted = Boolean(flag);
    this.applyMasterVolume();
    return this.muted;
  }

  async renderAudioOffline(
    startS = 0,
    endS = this.durationS(),
    options: { sampleRate?: number; channels?: number } = {},
  ): Promise<AudioBuffer | null> {
    const program = this.requireCompiledAudioProgram();
    const sampleRate = options.sampleRate ?? program.sampleRate;
    const channels = options.channels ?? 2;
    if (sampleRate !== program.sampleRate || channels !== 2) {
      throw new Error(
        `compiled audio requires ${program.sampleRate} Hz stereo, got ${sampleRate} Hz/${channels} channels`,
      );
    }
    const start = clamp(startS, 0, this.durationS());
    const end = clamp(endS, start, this.durationS());
    if (end <= start || !this.hasAudio()) return null;
    const startSample = this.sampleAtSeconds(start, true);
    const endSample = this.sampleAtSeconds(end, true);
    if (endSample <= startSample) return null;
    const frames = endSample - startSample;
    const offline = new OfflineAudioContext(2, frames, sampleRate);
    const buffer = await this.renderCompiledAudioBuffer(
      offline,
      startSample,
      endSample,
    );
    this.scheduleAudioGraph(
      offline,
      { buffer, startSample },
      start,
      offline.destination,
    );
    return offline.startRendering();
  }

  async replaceRenderPackage(next: RenderPackageReplacement) {
    if (this.closed) throw new Error("browser player is closed");
    if (!next?.timelineJson)
      throw new Error("replaceRenderPackage requires timelineJson");
    const resume = this.playing;
    this.pause();
    if (this.renderInFlight) await this.renderInFlight.catch(() => undefined);
    if (this.renderTail) await this.renderTail;
    await this.audioTail.catch(() => undefined);
    let staged: StagedRenderPackage;
    const stageStarted = performance.now();
    try {
      staged = await this.stageRenderPackage(next);
    } catch (error) {
      if (resume) {
        try {
          await this.play();
        } catch {
          /* preserve the admission failure */
        }
      }
      throw error;
    }
    if (next.isCurrent && !next.isCurrent()) {
      staged.compositor.dispose();
      staged.planner.close();
      staged.engine.free();
      if (resume) await this.play();
      return undefined;
    }
    this.commitStagedRenderPackage(staged);
    performance.mark("valle-player-package-stage", {
      detail: {
        mode: "replace",
        durationMs: performance.now() - stageStarted,
      },
    });
    this.timeS = clamp(this.timeS, 0, this.lastFrameTimeS());
    const frameStarted = performance.now();
    const rendered = await this.seek(this.timeS);
    performance.mark("valle-player-first-frame", {
      detail: {
        mode: "replace",
        durationMs: performance.now() - frameStarted,
      },
    });
    if (resume) await this.play();
    return rendered;
  }

  /** Validate one complete generated document with the same Rust constructor used by Project. */
  canonicalizeTimelineDocument(
    timeline: TimelineDocument,
  ): CanonicalTimelineDocument {
    if (!this.wasm)
      throw new Error(
        "Timeline canonical validation requires a loaded player runtime",
      );
    return canonicalizeTimelineDocumentWithWasm(this.wasm, timeline);
  }

  async videoKeyframeThumbnails(
    assetId: string,
    { height = 26, maxCount = 60 }: { height?: number; maxCount?: number } = {},
  ) {
    const asset = this.requireAsset(assetId, "video");
    const blob = await this.videoBlobs.get(
      this.assetUrl(asset),
      contentDigestHex(asset.contentDigest, "video digest"),
    );
    this.stats.videoThumbExtracts += 1;
    return keyframeThumbnails({ blob, height, maxCount });
  }

  async audioPeaks(
    assetId: string,
    { buckets = 2000 }: { buckets?: number } = {},
  ) {
    if (!Number.isSafeInteger(buckets) || buckets < 1) throw new RangeError("audio peak buckets must be a positive integer");
    this.runtimeAbort.signal.throwIfAborted();
    const work = this.readAudioPeaks(assetId, buckets);
    this.audioPeakTasks.add(work);
    try { return await work; }
    finally { this.audioPeakTasks.delete(work); }
  }

  private async readAudioPeaks(assetId: string, buckets: number) {
    const asset = this.requireAsset(assetId, "audio");
    const bytes = await this.fetchAssetBytes(asset);
    this.stats.audioPeaksDecodes += 1;
    const context = new OfflineAudioContext(1, 1, 8000);
    const reader = await BrowserAudioWindowReader.open({ buffer: bytes });
    let outputMemory: Awaited<ReturnType<BrowserRuntimeBudget["acquireKernel"]>> | undefined;
    try {
      const length = Math.round(reader.duration * context.sampleRate);
      if (!Number.isSafeInteger(length) || length < 1) throw new Error("audio track has no playable samples");
      const windowBytes = reader.workingBytes(context.sampleRate);
      if (windowBytes + buckets * 4 > this.runtimeBudget.memoryBytes) throw new RangeError("audio peaks exceed runtime memory budget");
      outputMemory = await this.runtimeBudget.acquireKernel(windowBytes + buckets * 4, this.runtimeAbort.signal);
      const peaks = new Float32Array(buckets);
      for (let start = 0; start < length; start += context.sampleRate) {
        let releaseCpu: (() => void) | undefined;
        try {
          releaseCpu = await this.runtimeBudget.acquireCpu("prefetch", this.runtimeAbort.signal);
          const end = Math.min(length, start + context.sampleRate);
          const source = (await reader.read(context, start, end)).getChannelData(0);
          this.runtimeAbort.signal.throwIfAborted();
          for (let index = 0; index < source.length; index++) {
            // Preserve the old floor(bucket * length / buckets) interval boundaries,
            // including empty buckets when there are fewer samples than buckets.
            const bucket = Math.ceil((start + index + 1) * buckets / length) - 1;
            peaks[bucket] = Math.max(peaks[bucket]!, Math.abs(source[index]!));
          }
        } finally { releaseCpu?.(); }
      }
      return { peaks, durationS: length / context.sampleRate };
    } finally {
      outputMemory?.release();
      reader.close();
    }
  }

  async motionInspection(
    clipId: string,
    canvasX: number,
    canvasY: number,
  ): Promise<{
    object: { semanticAddress: string; nodeId: number } | null;
    compositionFrame: number;
    sourceFrame: number;
    boxes: Record<string, [number, number, number, number]>;
  }> {
    const frame = this.frameAtSeconds(this.timeS);
    if (this.inspectedFrame !== frame) await this.renderFrame(frame);
    const inspection = this.frameInspection.motion.find(
      (value) => value.clipId === clipId,
    );
    if (!inspection) {
      const active =
        this.frameInspection.motion.map((value) => value.clipId).join(", ") ||
        "none";
      throw new Error(
        `motion clip '${clipId}' is inactive at frame ${frame}; active Motion clips: ${active}`,
      );
    }
    return {
      object: await this.pickScene3d(canvasX, canvasY, clipId),
      compositionFrame: inspection.compositionFrame,
      sourceFrame: inspection.sourceFrame,
      boxes: inspection.boxes,
    };
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.pause();
    this.closed = true;
    this.runtimeAbort.abort();
    if (this.renderInFlight) await this.renderInFlight.catch(() => undefined);
    if (this.renderTail) await this.renderTail;
    await this.audioTail.catch(() => undefined);
    await Promise.allSettled(this.audioPeakTasks);
    this.resourceObjects.dispose();
    this.planner?.close();
    this.planner = null;
    this.runtimeBudget.unregister(this.memoryOwner);
    this.runtimeBudget.unregister(this.audioMemoryOwner);
    this.engine?.free();
    this.renderId = "";
    this.renderReceipt = null;
    this.compiledAudioProgram = null;
    this.stats.renderId = "";
    this.compositor?.dispose();
    this.scene3dDof?.dispose();
    this.scene3dDof = null;
    this.scene3dGpu?.dispose();
    this.scene3dGpu = null;
    this.scene3dGpuInit = null;
    this.gpuCircles?.dispose();
    this.gpuCircles = null;
    const closingGl = this.target?.canvas?.getContext("webgl2");
    if (closingGl) untrackGpuState(closingGl);
    this.currentScene3dRequests.clear();
    for (const ring of this.videoRings.values()) ring.close();
    this.target?.surface.delete();
    this.target?.context?.delete();
    this.target = null;
    this.warmedPreviewTarget = null;
    this.warmedPreviewRenderId = null;
    this.scene3dResourceTasks.clear();
    this.videoRings.clear();
    this.assetBytes.clear();
    this.videoBlobs.clear();
    this.audioBuffers.clear();
    for (const reader of this.audioReaders.values()) reader.close();
    this.audioReaders.clear();
    this.fontBytes.clear();
    this.modelBytes.clear();
    this.environmentBytes.clear();
    this.shaderBytes.clear();
    this.hitRects = [];
    this.frameInspection = {
      renderId: "",
      compositionFrame: -1,
      hitRects: [],
      motion: [],
      scene3d: [],
    };
    this.inspectedFrame = null;
    this.inspectionScaleX = 1;
    this.inspectionScaleY = 1;
  }

  private currentRenderPackage(): RenderPackageReplacement {
    return {
      fixedPackageManifestJson: this.fixedPackageManifestJson,
      timelineJson: this.timelineJson,
      resourceManifestJson: this.resourceManifestJson,
      verifiedBindingBundleJson: this.verifiedBindingBundleJson,
      assets: this.assets,
    };
  }

  private async stageRenderPackage(
    next: RenderPackageReplacement,
  ): Promise<StagedRenderPackage> {
    const markPhase = (phase: string, started: number): void => {
      performance.mark("valle-player-package-phase", {
        detail: {
          phase,
          durationMs: performance.now() - started,
        },
      });
    };
    if (this.loadMotionFonts) {
      const fontsStarted = performance.now();
      const fontsJson = await this.loadMotionFonts();
      const selected = JSON.parse(
        this.wasm.with_motion_fonts(
          next.fixedPackageManifestJson,
          next.timelineJson,
          next.resourceManifestJson,
          next.verifiedBindingBundleJson,
          fontsJson,
        ),
      ) as RenderPackageReplacement;
      next = { ...next, ...selected };
      markPhase("fonts", fontsStarted);
    }
    const canonicalStarted = performance.now();
    const canonical = this.canonicalizeTimelineDocument(
      JSON.parse(next.timelineJson) as TimelineDocument,
    );
    if (next.timelineJson !== canonical.timelineJson) {
      throw new Error(
        "timelineJson must contain the exact canonical Timeline bytes",
      );
    }
    markPhase("canonical", canonicalStarted);
    const fixedPackageManifestJson = next.fixedPackageManifestJson;
    const resourceManifestJson = next.resourceManifestJson;
    const verifiedBindingBundleJson = next.verifiedBindingBundleJson;
    const resourceManifest = JSON.parse(
      resourceManifestJson,
    ) as ResourceManifest;
    const assets = (next.assets ?? this.assets).map((asset) => ({ ...asset }));
    const admittedAssets = admitBrowserAssets(assets, resourceManifest);
    let frozenResources: FrozenBundleResources;
    const engine = new this.wasm.ProductEngine();
    let planner: ProductFramePlannerPool | null = null;
    let compositor: CanvasKitExecutor | null = null;
    try {
      const bootstrap = emptyProductEngineBootstrap(
        this.wasmModuleUrl,
        this.wasmUrl,
        fixedPackageManifestJson,
        canonical.timelineJson,
        resourceManifestJson,
        verifiedBindingBundleJson,
      );
      bootstrap.gpuScene3d = this.gpu && this.renderMode === "realtime";
      const openStarted = performance.now();
      planner = new ProductFramePlannerPool({
        workerUrl: this.runtimeAssets.workers.productFrame,
        runtimeBudget: this.runtimeBudget,
        priority: this.runtimePurpose === "export" ? "export" : "playback",
        maxReadyBytes: Math.max(1024, Math.min(96 * 1024 ** 2, Math.floor(this.runtimeBudget.memoryBytes / 16))),
        allocateGeneration: () => this.allocateGeneration(),
      });
      await planner.init(bootstrap);
      const snapshot = planner.initialization;
      if (!snapshot) throw new Error("Product worker did not return the admitted package snapshot");
      const receipt = parseProductRenderReceipt(JSON.stringify(snapshot.receipt));
      bootstrap.expectedRenderId = receipt.renderId;
      markPhase("open", openStarted);
      frozenResources = indexCompiledExecutionResources(snapshot);
      const audioProgram = parseCompiledAudioProgram(JSON.parse(snapshot.audioProgramJson));
      if (audioProgram.sampleRate !== receipt.sampleRate || audioProgram.sampleCount !== receipt.sampleCount) {
        throw new Error("compiled audio program does not match the opened render receipt");
      }
      // The UI engine owns only stateless GPU recipes and the common clock, never a Timeline.
      const runtime = planner;
      const kernels = new Proxy(engine, { get(target, property) {
        if (typeof property === "string" && ((property.startsWith("apply_") && property.endsWith("_f16")) || property.startsWith("transform_srgb_preview_"))) {
          return (...args: unknown[]) => runtime.call<Uint8Array>(property as ProductRuntimeMethod, args);
        }
        const value = Reflect.get(target, property);
        return typeof value === "function" ? value.bind(target) : value;
      } });
      compositor = new CanvasKitExecutor(this.CanvasKit, kernels);
      return {
        fixedPackageManifestJson,
        timelineJson: canonical.timelineJson,
        timeline: canonical.timeline,
        resourceManifestJson,
        resourceManifest,
        verifiedBindingBundleJson,
        assets,
        admittedAssets,
        frozenResources,
        engine,
        planner,
        bootstrap,
        compositor,
        receipt,
        audioProgram,
      };
    } catch (error) {
      compositor?.dispose();
      planner?.close();
      engine.free();
      throw error;
    }
  }

  private commitStagedRenderPackage(staged: StagedRenderPackage): void {
    const previousRuntime = {
      engine: this.engine as BrowserProductEngineWire | undefined,
      planner: this.planner,
      executor: this.compositor as CanvasKitExecutor | undefined,
    };

    this.scene3dGpu?.dispose();
    this.scene3dGpu = null;
    this.scene3dGpuInit = null;
    this.scene3dDof?.dispose();
    this.scene3dDof = null;
    this.currentScene3dRequests?.clear();
    this.fixedPackageManifestJson = staged.fixedPackageManifestJson;
    this.timelineJson = staged.timelineJson;
    this.timeline = staged.timeline;
    this.resourceManifestJson = staged.resourceManifestJson;
    this.resourceManifest = staged.resourceManifest;
    this.verifiedBindingBundleJson = staged.verifiedBindingBundleJson;
    this.assets = staged.assets;
    replaceMap(this.assetsById, staged.admittedAssets.byId);
    replaceMap(this.assetByDigest, staged.admittedAssets.byDigest);
    replaceMap(this.fontBytes, staged.frozenResources.fonts);
    replaceMap(this.modelBytes, staged.frozenResources.models);
    replaceMap(this.environmentBytes, staged.frozenResources.environments);
    replaceMap(this.shaderBytes, staged.frozenResources.shaders);
    this.engine = staged.engine;
    this.planner = staged.planner;
    this.bootstrap = staged.bootstrap;
    this.compositor = staged.compositor;
    this.renderId = staged.receipt.renderId;
    this.renderReceipt = staged.receipt;
    this.compiledAudioProgram = staged.audioProgram;
    this.stats.renderId = staged.receipt.renderId;
    this.stats.fontsRegistered = staged.frozenResources.fonts.size;

    if (
      previousRuntime.engine ||
      previousRuntime.planner ||
      previousRuntime.executor
    ) {
      this.resourceObjects.invalidateGeneration();
    }
    for (const ring of this.videoRings.values()) ring.close();
    this.scene3dResourceTasks.clear();
    this.videoRings.clear();
    this.assetBytes.clear();
    this.videoBlobs.clear();
    this.audioBuffers.clear();
    for (const reader of this.audioReaders.values()) reader.close();
    this.audioReaders.clear();
    this.playbackFrame = null;
    this.hitRects = [];
    this.frameInspection = {
      renderId: "",
      compositionFrame: -1,
      hitRects: [],
      motion: [],
      scene3d: [],
    };
    this.inspectedFrame = null;
    this.inspectionScaleX = 1;
    this.inspectionScaleY = 1;
    this.lastRender = null;

    previousRuntime.executor?.dispose();
    previousRuntime.planner?.close();
    previousRuntime.engine?.free();
    this.syncPlannerStats();
  }

  private async fulfillRequests(
    packet: Uint8Array,
    gpu: boolean,
    projected?: Record<string, Float32Array>,
  ): Promise<FulfilledFrame> {
    const decoded = await decodePackedAbi(packet, RESOURCE_REQUESTS_ABI);
    if (!Array.isArray(decoded))
      throw new Error("ResourceRequestSet must decode to an array");
    this.resourceObjects.beginFrame();
    try {
      const objects = new Map<number, CanvasKitExternalObject>();
      const scene3dRequests: NonNullable<FulfilledFrame["scene3dRequests"]> =
        new Map();
      const dispose: Array<() => void> = [];
      const requests = decoded.map((value) => {
        const request = record(value, "resource request") as PackedValue & Wire;
        if (
          record(request.expected, "resource expected").kind === "scene3d"
        ) {
          const key = record(request.key, "Scene3D key"),
            payload = record(request.payload, "Scene3D payload");
          const canonical = packedByteArray(
            payload.canonical_request,
            "Scene3D canonical request",
          );
          const frame = JSON.parse(
            new TextDecoder().decode(canonical),
          ) as Scene3dGpuRequest;
          scene3dRequests.set(
            contentDigestWire(key.content, "Scene3D content"),
            {
              canonical,
              topology: contentDigestWire(
                record(key.interpretation, "Scene3D interpretation")
                  .topology_digest,
                "Scene3D topology",
              ),
              width: frame.width,
              height: frame.height,
            },
          );
        }
        return {
          request,
          handle: exactInteger(request.handle, "resource handle"),
          identity: resourceRequestCacheIdentity(request),
        };
      });
      const groups = new Map<
        string,
        {
          request: PackedValue & Wire;
          cached: CachedBrowserResource | undefined;
          produced: ProducedBrowserResource | null;
        }
      >();
      for (const { identity, request } of requests) {
        if (!groups.has(identity)) {
          groups.set(identity, { request, cached: undefined, produced: null });
        }
      }
      for (const [identity, group] of groups) {
        group.cached = this.resourceObjects.lookup(identity);
      }
      const produced = await Promise.all(
        [...groups.values()].map(async (group) => {
          if (group.cached !== undefined) return null;
          try {
            return {
              value: await this.fulfillRequestUncached(
                group.request,
                gpu,
                projected,
              ),
            } as const;
          } catch (error) {
            return { error } as const;
          }
        }),
      );
      const failure = produced.find(
        (result): result is { readonly error: unknown } =>
          result !== null && "error" in result,
      );
      if (failure) {
        disposeAll(
          produced.flatMap((result) =>
            result && "value" in result && result.value
              ? [result.value.dispose]
              : [],
          ),
        );
        throw failure.error;
      }
      for (const [index, group] of [...groups.values()].entries()) {
        const generated = produced[index];
        group.produced =
          generated && "value" in generated && generated.value
            ? generated.value
            : null;
      }

      const resolved = new Map<string, CachedBrowserResource>();
      const pendingDisposals = new Set(
        [...groups.values()].flatMap((group) =>
          group.produced ? [group.produced.dispose] : [],
        ),
      );
      try {
        for (const [identity, group] of groups) {
          const value = group.cached ?? group.produced;
          if (!value) throw new Error("resource fulfillment result is missing");
          if (group.cached === undefined) {
            const owned = value as ProducedBrowserResource;
            const cachedValue: CachedBrowserResource = {
              object: owned.object,
              videoFrames: owned.videoFrames,
              videoTextureFrames: owned.videoTextureFrames,
            };
            const retained = this.resourceObjects.insert(
              identity,
              cachedValue,
              owned.bytes,
              owned.dispose,
            );
            if (retained) pendingDisposals.delete(owned.dispose);
            else dispose.push(owned.dispose);
            resolved.set(identity, cachedValue);
          } else {
            resolved.set(identity, group.cached);
          }
        }

        let videoFrames = 0;
        let videoTextureFrames = 0;
        const seen = new Set<string>();
        for (const { handle, identity } of requests) {
          const value = resolved.get(identity);
          if (!value) throw new Error("resource fulfillment result is missing");
          if (seen.has(identity))
            this.resourceObjects.recordFrameReuse(identity);
          else seen.add(identity);
          if (objects.has(handle))
            throw new Error(`resource handle ${handle} is duplicated`);
          objects.set(handle, value.object);
          videoFrames += value.videoFrames;
          videoTextureFrames += value.videoTextureFrames;
        }
        const resourceCache = this.resourceObjects.finishFrame(requests.length);
        return {
          objects,
          dispose,
          videoFrames,
          videoTextureFrames,
          resourceCache,
          scene3dRequests,
        };
      } catch (error) {
        disposeAll(pendingDisposals);
        throw error;
      }
    } catch (error) {
      this.resourceObjects.releaseFrame();
      throw error;
    }
  }

  private async fulfillRequestUncached(
    request: PackedValue & Wire,
    gpu: boolean,
    projected?: Record<string, Float32Array>,
  ): Promise<ProducedBrowserResource> {
    const key = record(request.key, "resource key") as PackedValue & Wire;
    const expected = record(request.expected, "resource expected descriptor");
    const interpretation = record(
      key.interpretation,
      "resource interpretation",
    );
    const contentWire = contentDigestWire(key.content, "resource key content");
    const content = contentWire.slice("sha256:".length);
    const empty = { videoFrames: 0, videoTextureFrames: 0 };
    if (expected.kind === "visualFrame") {
      assertCanvaskitImportLayout(expected);
      const asset = this.assetByDigest.get(content);
      if (!asset)
        throw new Error(
          `visual resource ${content} has no admitted browser asset`,
        );
      const extent = record(expected.extent, "visual resource extent");
      const bytes = pixelBytes(extent.width, extent.height, 4);
      const sample = record(request.sample, "resource sample");
      if (asset.type === "video") {
        const time = rationalSeconds(sample.time);
        const result = await this.videoImage(asset, time, gpu);
        return {
          object: { key, kind: "visual", image: result.image },
          bytes,
          dispose: result.dispose,
          videoFrames: 1,
          videoTextureFrames: Number(result.texture),
        };
      }
      const image = await this.staticImage(asset);
      return {
        object: { key, kind: "visual", image },
        bytes,
        dispose: () => image.delete(),
        ...empty,
      };
    }
    if (expected.kind === "dataTexture") {
      if (interpretation.kind !== "dataTexture")
        throw new Error("data texture request has wrong interpretation");
      const asset = this.assetByDigest.get(content);
      if (!asset || asset.type !== "image")
        throw new Error(`data texture ${content} has no admitted image asset`);
      const extent = record(expected.extent, "data texture extent");
      const width = Number(extent.width),
        height = Number(extent.height);
      const encoded = await this.fetchAssetBytes(asset, content);
      const pixels = await this.runtimeCall<Uint8Array>("decode_shader_data_texture",
        contentWire,
        encoded,
        width,
        height,
      );
      const bytes = pixelBytes(width, height, 8);
      if (pixels.byteLength !== bytes)
        throw new Error("data texture decoder returned invalid storage");
      const image = this.CanvasKit.MakeImage(
        {
          width: width * 2,
          height: height * 4,
          colorType: this.CanvasKit.ColorType.Alpha_8,
          alphaType: this.CanvasKit.AlphaType.Premul,
          colorSpace: this.CanvasKit.ColorSpace.SRGB,
        },
        pixels,
        width * 2,
      );
      if (!image)
        throw new Error(`CanvasKit cannot wrap data texture ${content}`);
      return {
        object: { key, kind: "dataTexture", image },
        bytes,
        dispose: () => image.delete(),
        ...empty,
      };
    }
    if (expected.kind === "fontBytes") {
      if (interpretation.kind !== "fontFace")
        throw new Error("font request has wrong interpretation");
      const bytes = this.fontBytes.get(content);
      if (!bytes) throw new Error(`font bytes ${content} are not registered`);
      return {
        object: { key, kind: "font", bytes },
        bytes: bytes.byteLength,
        dispose: noop,
        ...empty,
      };
    }
    if (expected.kind === "runtimeShader") {
      const abi = contentDigestHex(
        interpretation.abi_digest,
        "runtime shader ABI digest",
      );
      const bytes = this.shaderBytes.get(`${content}:${abi}`);
      if (!bytes)
        throw new Error(`runtime shader ${content}:${abi} is not registered`);
      return {
        object: { key, kind: "runtimeShader", bytes },
        bytes: bytes.byteLength,
        dispose: noop,
        ...empty,
      };
    }
    if (expected.kind === "scene3d") {
      const payload = record(request.payload, "Scene3D request payload");
      const payloadKeys = Object.keys(payload).sort();
      if (
        payload.kind !== "scene3dFrame" ||
        payloadKeys.length !== 2 ||
        payloadKeys[0] !== "canonical_request" ||
        payloadKeys[1] !== "kind"
      ) {
        throw new Error("Scene3D request has an invalid payload shape");
      }
      const canonicalRequest = packedByteArray(
        payload.canonical_request,
        "Scene3D canonical request",
      );
      // A projected packet has already passed Rust admission on a stateless kernel worker.
      // Supported packets have no external model/texture dependencies.
      if (!projected?.[contentWire]) await this.ensureScene3dResources(canonicalRequest);
      const topology = contentDigestWire(
        interpretation.topology_digest,
        "Scene3D topology digest",
      );
      if (
        this.renderMode === "realtime" &&
        gpu &&
        this.target?.context &&
        this.target.canvas &&
        this.planner
      ) {
        const target = this.target,
          activeEngine = this.engine;
        this.scene3dGpuInit ??= createScene3dGpu(
          this.CanvasKit,
          target.canvas!,
          target.context!,
          target.surface,
          () =>
            this.target === target &&
            this.engine === activeEngine &&
            !this.closed,
        ).then((renderer) => {
          if (
            this.engine !== activeEngine ||
            this.target !== target ||
            this.closed
          ) {
            renderer?.dispose();
            return null;
          }
          this.scene3dGpu = renderer;
          return renderer;
        });
        const renderer = await this.scene3dGpuInit;
        const value = JSON.parse(
          new TextDecoder().decode(canonicalRequest),
        ) as Scene3dGpuRequest;
        if (renderer?.accepts(value)) {
          if (!projected?.[contentWire]) await this.runtimeCall<void>("admit_scene3d_gpu_request",
            contentWire,
            topology,
            canonicalRequest,
          );
          value.lightingInput =
            projected?.[contentWire] ??
            await this.runtimeCall<Float32Array>("scene3d_gpu_lighting_input", canonicalRequest);
          const image = await renderer.render(value);
          return {
            object: { key, kind: "scene3d", image },
            bytes: pixelBytes(value.width, value.height, 8),
            dispose: () => image.delete(),
            ...empty,
          };
        }
      }
      if (
        this.renderMode === "realtime" &&
        gpu &&
        this.target?.context &&
        this.planner
      ) {
        const packet = await this.runtimeCall<Uint8Array>("render_scene3d_request_deferred",
          contentWire,
          topology,
          canonicalRequest,
        );
        const input = decodeDeferredScene3dDof(packet);
        this.scene3dDof ??= new CanvasKitScene3dDof(
          this.CanvasKit,
          this.target.context,
        );
        const image = this.scene3dDof.render(input);
        return {
          object: { key, kind: "scene3d", image },
          bytes: pixelBytes(input.width, input.height, 8),
          dispose: () => image.delete(),
          ...empty,
        };
      }
      const rgba = await this.runtimeCall<Uint8Array>("render_scene3d_request",
        contentWire,
        topology,
        canonicalRequest,
      );
      const { width, height } = JSON.parse(new TextDecoder().decode(canonicalRequest)) as { width: number; height: number };
      const bytes = pixelBytes(width, height, 8);
      if (rgba.byteLength !== bytes) {
        throw new Error(`Scene3D ${content} returned an invalid RGBA16F plane`);
      }
      const image = this.CanvasKit.MakeImage(
        {
          width,
          height,
          colorType: this.CanvasKit.ColorType.RGBA_F16,
          alphaType: this.CanvasKit.AlphaType.Premul,
          colorSpace: this.CanvasKit.ColorSpace.SRGB,
        },
        rgba,
        width * 8,
      );
      if (!image)
        throw new Error(`CanvasKit cannot wrap Scene3D frame ${content}`);
      return {
        object: { key, kind: "scene3d", image },
        bytes,
        dispose: () => image.delete(),
        ...empty,
      };
    }
    throw new Error(
      `unsupported browser resource descriptor '${String(expected.kind)}'`,
    );
  }

  private runtimeCall<T>(method: ProductRuntimeMethod, ...args: unknown[]): Promise<T> {
    return this.requirePlanner().call<T>(method, args);
  }

  private async ensureScene3dResources(
    canonicalRequest: Uint8Array,
  ): Promise<void> {
    const needs = JSON.parse(
      await this.runtimeCall<string>("scene3d_resource_needs_json", canonicalRequest),
    ) as {
      models: string[];
      textures: { digest: string; role: "color" | "data" }[];
      environments: string[];
    };
    await Promise.all([
      ...needs.environments.map((digest) =>
        this.ensureScene3dResource("environment", digest),
      ),
      ...needs.models.map((digest) =>
        this.ensureScene3dResource("model", digest),
      ),
      ...needs.textures.map(({ digest, role }) =>
        this.ensureScene3dResource("texture", digest, role),
      ),
    ]);
  }

  private async ensureScene3dResource(
    kind: "model" | "texture" | "environment",
    declaredDigest: string,
    role?: "color" | "data",
  ): Promise<void> {
    const digestWire = contentDigestWire(
      declaredDigest,
      `Scene3D ${kind} digest`,
    );
    const digest = digestWire.slice("sha256:".length);
    const key = `${kind}:${digest}:${role ?? ""}`;
    const existing = this.scene3dResourceTasks.get(key);
    if (existing) return existing;
    const task = (async () => {
      if (kind === "environment") {
        const bytes = this.environmentBytes.get(digest);
        if (!bytes)
          throw new Error(
            `Scene3D environment ${digest} has no admitted frozen payload`,
          );
        await this.runtimeCall<void>("register_scene3d_environment", digestWire, bytes);
        return;
      }
      if (kind === "model") {
        const bytes = this.modelBytes.get(digest);
        if (!bytes)
          throw new Error(
            `Scene3D model ${digest} has no admitted frozen payload`,
          );
        await this.runtimeCall<void>("register_scene3d_model", digestWire, bytes);
        return;
      }
      const asset = this.assetByDigest.get(digest);
      if (!asset)
        throw new Error(
          `Scene3D ${kind} ${digest} has no admitted browser asset`,
        );
      const bytes = await this.fetchAssetBytes(asset, digest);
      if (role !== "color" && role !== "data")
        throw new Error("Scene3D texture requires a color/data role");
      await this.runtimeCall<void>("register_scene3d_texture", digestWire, bytes, role);
    })();
    this.scene3dResourceTasks.set(key, task);
    try {
      await task;
    } catch (error) {
      this.scene3dResourceTasks.delete(key);
      throw error;
    }
  }

  private async staticImage(asset: AdmittedBrowserAsset): Promise<Image> {
    const image = this.CanvasKit.MakeImageFromEncoded(
      await this.fetchAssetBytes(asset),
    );
    if (!image)
      throw new Error(`CanvasKit cannot decode image asset '${asset.id}'`);
    return image;
  }

  private async videoImage(
    asset: AdmittedBrowserAsset,
    timeS: number,
    gpu: boolean,
  ): Promise<{ image: Image; texture: boolean; dispose: () => void }> {
    const started = performance.now();
    this.videoRingUsers.set(
      asset.id,
      (this.videoRingUsers.get(asset.id) ?? 0) + 1,
    );
    let frame: VideoFrame;
    try {
      const ring = await this.videoRing(asset);
      frame = await ring.frameAt(timeS);
    } finally {
      const users = (this.videoRingUsers.get(asset.id) ?? 1) - 1;
      if (users) this.videoRingUsers.set(asset.id, users);
      else this.videoRingUsers.delete(asset.id);
      this.trimVideoRings();
    }
    this.stats.perfVideoDecodeMs += performance.now() - started;
    if (gpu) {
      const image = makeLazyVideoFrameImage(this.CanvasKit, frame, {
        width: frame.displayWidth,
        height: frame.displayHeight,
        colorType: this.CanvasKit.ColorType.RGBA_8888,
        alphaType: this.CanvasKit.AlphaType.Opaque,
      });
      if (image)
        return {
          image,
          texture: true,
          dispose: () => {
            image.delete();
            frame.close();
          },
        };
    }
    try {
      const rgba = await videoFrameToRgba(frame);
      const image = this.CanvasKit.MakeImage(
        {
          width: rgba.width,
          height: rgba.height,
          colorType: this.CanvasKit.ColorType.RGBA_8888,
          alphaType: this.CanvasKit.AlphaType.Unpremul,
          colorSpace: this.CanvasKit.ColorSpace.SRGB,
        },
        rgba.data,
        rgba.width * 4,
      );
      if (!image)
        throw new Error(`CanvasKit cannot wrap video frame for '${asset.id}'`);
      return { image, texture: false, dispose: () => image.delete() };
    } finally {
      frame.close();
    }
  }

  private async videoRing(asset: AdmittedBrowserAsset): Promise<DecoderRing> {
    let ring = this.videoRings.get(asset.id);
    if (!ring) {
      const blob = await this.videoBlobs.get(
        this.assetUrl(asset),
        contentDigestHex(asset.contentDigest, "video digest"),
      );
      if (this.closed) throw new Error("player closed during video load");
      ring = this.videoRings.get(asset.id);
      if (!ring) {
        ring = createDecoderRing({ blob });
        this.videoRings.set(asset.id, ring);
      }
    }
    this.videoRings.delete(asset.id);
    this.videoRings.set(asset.id, ring);
    // Retire idle decoders before awaiting the new source's first frame. Otherwise an
    // entire extra decode window remains resident during every asynchronous source switch.
    this.trimVideoRings();
    return ring;
  }

  private trimVideoRings(): void {
    for (const [id, ring] of this.videoRings) {
      if (this.videoRings.size <= 2) break;
      if (this.videoRingUsers.has(id)) continue;
      ring.close();
      this.videoRings.delete(id);
    }
  }

  private acquireTarget(
    width: number,
    height: number,
    forceCpu: boolean,
  ): TargetSurface {
    const gpu = this.gpu && !forceCpu && typeof document !== "undefined";
    const key = `${gpu ? "gpu" : "cpu"}:${width}x${height}`;
    if (this.target?.key === key) return this.target;
    this.warmedPreviewTarget = null;
    if (this.target) this.resourceObjects.invalidateGeneration();
    this.scene3dDof?.dispose();
    this.scene3dDof = null;
    this.scene3dGpu?.dispose();
    this.scene3dGpu = null;
    this.scene3dGpuInit = null;
    this.gpuCircles?.dispose();
    this.gpuCircles = null;
    const retiredGl = this.target?.canvas?.getContext("webgl2");
    if (retiredGl) untrackGpuState(retiredGl);
    this.target?.surface.delete();
    this.target?.context?.delete();
    if (gpu) {
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const handle = this.CanvasKit.GetWebGLContext(canvas, {
        preserveDrawingBuffer: 1,
      });
      const context = handle ? this.CanvasKit.MakeWebGLContext(handle) : null;
      const surface = context
        ? this.CanvasKit.MakeOnScreenGLSurface(
            context,
            width,
            height,
            this.CanvasKit.ColorSpace.SRGB,
          )
        : null;
      if (surface && context) {
        this.target = { key, surface, gpu: true, context, canvas };
        const gl = canvas.getContext("webgl2");
        if (
          this.renderMode === "realtime" &&
          gl?.getExtension("EXT_color_buffer_float")
        ) {
          this.gpuCircles = new GpuCircles({
            CanvasKit: this.CanvasKit,
            context,
            gl,
            resetContext: () => resetCanvasKitContext(surface),
          });
        }
        return this.target;
      }
      context?.delete();
    }
    const surface = this.CanvasKit.MakeSurface(width, height);
    if (!surface)
      throw new Error(
        `CanvasKit cannot allocate product target ${width}x${height}`,
      );
    this.target = {
      key: `cpu:${width}x${height}`,
      surface,
      gpu: false,
      context: null,
      canvas: null,
    };
    return this.target;
  }

  private presentTarget(
    target: TargetSurface,
    width: number,
    height: number,
  ): void {
    if (!this.canvas) return;
    if (this.canvas.width !== width) this.canvas.width = width;
    if (this.canvas.height !== height) this.canvas.height = height;
    const context = this.canvas.getContext("2d");
    if (!context) throw new Error("visible canvas 2D context is unavailable");
    if (target.gpu && target.canvas) {
      // The presented frame replaces its predecessor, including transparent pixels.
      context.clearRect(0, 0, width, height);
      context.drawImage(target.canvas, 0, 0);
      return;
    }
    const pixels = target.surface.getCanvas().readPixels(0, 0, {
      width,
      height,
      colorType: this.CanvasKit.ColorType.RGBA_8888,
      alphaType: this.CanvasKit.AlphaType.Unpremul,
      colorSpace: this.CanvasKit.ColorSpace.SRGB,
    });
    if (!(pixels instanceof Uint8Array))
      throw new Error("CanvasKit target readback failed");
    context.putImageData(
      new ImageData(Uint8ClampedArray.from(pixels), width, height),
      0,
      0,
    );
  }

  private encodeTargetPng(surface: Surface): Uint8Array {
    const image = surface.makeImageSnapshot();
    try {
      const bytes = image.encodeToBytes(this.CanvasKit.ImageFormat.PNG, 100);
      if (!bytes) throw new Error("CanvasKit PNG encoding failed");
      // CanvasKit may back the returned view with WASM-owned storage tied to the snapshot. The
      // public capture result must remain immutable after `image.delete()` and across subsequent
      // encodes, so detach it into JavaScript-owned bytes before releasing the image.
      return Uint8Array.from(bytes);
    } finally {
      image.delete();
    }
  }

  private scheduleTick(): void {
    this.rafId = requestAnimationFrame(() => this.tick());
  }

  private tick(): void {
    if (!this.playing || this.closed) return;
    this.stats.rAFFrames += 1;
    const now = clamp(this.clock.now(), 0, this.lastFrameTimeS());
    this.timeS = now;
    this.onTimeUpdate?.(now);
    this.scheduleTick();
  }

  private async runPlaybackPump(generation: number): Promise<void> {
    if (this.previewBuffer)
      return this.runBufferedPlayback(generation, this.previewBuffer);
    if (this.renderMode === "realtime" && this.target?.gpu)
      return this.runDirectGpuPlayback(generation);
    const receipt = this.requireRenderReceipt();
    const fps = canonicalRationalNumber(receipt.frameRate, "frameRate");
    const lastFrame = receipt.frameCount - 1;
    const clockFrame = this.frameAtSeconds(this.timeS);
    let frame = Math.max(
      clockFrame,
      this.playbackFrame === null ? clockFrame : this.playbackFrame + 1,
    );
    while (
      this.playing &&
      !this.closed &&
      generation === this.playGeneration &&
      frame <= lastFrame
    ) {
      // Preview follows the media clock. After a slow frame, seek planning forward instead of
      // replaying an ever-growing backlog; exact renderFrame/seek and offline export keep every
      // requested frame. The clamped clock still lets us present the final frame once.
      frame = Math.max(
        frame,
        this.frameAtSeconds(clamp(this.clock.now(), 0, this.lastFrameTimeS())),
      );
      this.prefetchPlanningWindow(frame);
      const dueTime = frame / fps;
      while (
        this.playing &&
        !this.closed &&
        generation === this.playGeneration
      ) {
        const remainingMs = (dueTime - this.clock.now()) * 1000;
        if (remainingMs <= 0.5) break;
        await waitMilliseconds(Math.min(8, remainingMs));
      }
      if (!this.playing || this.closed || generation !== this.playGeneration)
        return;
      const job = this.renderFrame(frame).then(() => {
        this.playbackFrame = frame;
      });
      this.renderInFlight = job;
      try {
        await job;
      } finally {
        if (this.renderInFlight === job) this.renderInFlight = null;
      }
      frame += 1;
    }
    if (!this.playing || this.closed || generation !== this.playGeneration)
      return;
    this.pause();
    this.timeS = this.lastFrameTimeS();
    this.onTimeUpdate?.(this.timeS);
  }

  private async runDirectGpuPlayback(generation: number): Promise<void> {
    const receipt = this.requireRenderReceipt(), fps = canonicalRationalNumber(receipt.frameRate, "frameRate");
    const last = receipt.frameCount - 1;
    let frame = Math.max(this.frameAtSeconds(this.timeS), this.playbackFrame === null ? 0 : this.playbackFrame + 1);
    while (this.playing && !this.closed && generation === this.playGeneration && frame <= last) {
      frame = Math.max(frame, this.frameAtSeconds(clamp(this.clock.now(), 0, this.lastFrameTimeS())));
      this.prefetchPlanningWindow(frame);
      // Submit the next frame before its display deadline. GPU work then overlaps the media
      // clock's wait instead of starting after that deadline. There is exactly one pixel owner.
      const job = this.renderFrameDetailed(frame, {}, false, async completed => {
        while (this.playing && !this.closed && generation === this.playGeneration) {
          const remaining = (frame / fps - this.clock.now()) * 1000;
          if (remaining <= 0.5) break;
          await waitMilliseconds(Math.min(8, remaining));
        }
        if (!this.playing || this.closed || generation !== this.playGeneration) return;
        this.presentTarget(this.target!, completed.width, completed.height);
        this.publishInspection(completed);
        this.playbackFrame = frame;
      });
      this.renderInFlight = job;
      try { await job; }
      finally { if (this.renderInFlight === job) this.renderInFlight = null; }
      frame++;
    }
    if (this.playing && !this.closed && generation === this.playGeneration) {
      this.pause();this.timeS = this.lastFrameTimeS();this.onTimeUpdate?.(this.timeS);
    }
  }

  private async runBufferedPlayback(
    generation: number,
    buffer: PreviewBuffer<BufferedPreview>,
  ): Promise<void> {
    const receipt = this.requireRenderReceipt(),
      fps = canonicalRationalNumber(receipt.frameRate, "frameRate"),
      last = receipt.frameCount - 1;
    let frame = Math.max(
      this.frameAtSeconds(this.timeS),
      this.playbackFrame === null ? 0 : this.playbackFrame + 1,
    );
    try {
      while (
        this.playing &&
        !this.closed &&
        generation === this.playGeneration &&
        frame <= last
      ) {
        frame = Math.max(
          frame,
          this.frameAtSeconds(
            clamp(this.clock.now(), 0, this.lastFrameTimeS()),
          ),
        );
        const next = await buffer.take(frame);
        if (!next) return;
        const completed = next.value;
        frame = next.frame;
        // A sustained producer deficit is not a reason to keep a pixel queue. Retire its
        // in-flight owner, then let the direct pump demand the current clock frame.
        if (frame + 1 < this.frameAtSeconds(clamp(this.clock.now(), 0, this.lastFrameTimeS()))) {
          completed.bitmap.close();
          buffer.close();
          if (this.previewBuffer === buffer) this.previewBuffer = null;
          await buffer.producer;
          if (this.playing && !this.closed && generation === this.playGeneration)
            return this.runPlaybackPump(generation);
          return;
        }
        try {
          while (
            this.playing &&
            !this.closed &&
            generation === this.playGeneration
          ) {
            const remaining = (frame / fps - this.clock.now()) * 1000;
            if (remaining <= 0.5) break;
            await waitMilliseconds(Math.min(8, remaining));
          }
          if (
            !this.playing ||
            this.closed ||
            generation !== this.playGeneration
          )
            return;
          const canvas = this.canvas!,
            context = canvas.getContext("2d");
          if (!context)
            throw new Error("visible canvas 2D context is unavailable");
          if (canvas.width !== completed.width) canvas.width = completed.width;
          if (canvas.height !== completed.height)
            canvas.height = completed.height;
          context.clearRect(0, 0, canvas.width, canvas.height);
          context.drawImage(completed.bitmap, 0, 0);
          this.publishInspection(completed);
          this.playbackFrame = frame;
        } finally {
          completed.bitmap.close();
        }
        frame++;
      }
    } finally {
      buffer.close();
    }
    if (this.playing && !this.closed && generation === this.playGeneration) {
      this.pause();
      this.timeS = this.lastFrameTimeS();
      this.onTimeUpdate?.(this.timeS);
    }
  }

  private async prefetchAudio(
    mediaTime: number,
    context: BaseAudioContext | null,
  ): Promise<CompiledAudioPlayback | null> {
    if (!context || !this.hasAudio()) return null;
    const program = this.requireCompiledAudioProgram();
    if (context.sampleRate !== program.sampleRate) {
      throw new Error(
        `WebAudio context is ${context.sampleRate} Hz; active render requires ${program.sampleRate} Hz`,
      );
    }
    const startSample = this.sampleAtSeconds(mediaTime, true);
    if (startSample >= program.sampleCount) return null;
    try {
      return {
        buffer: await this.renderCompiledAudioBuffer(
          context,
          startSample,
          Math.min(program.sampleCount, startSample + program.sampleRate),
        ),
        startSample,
      };
    } catch (error) {
      this.stats.audioDecodeErrors += 1;
      throw error;
    }
  }

  private async audioReaderForDigest(source: AudioSourceRequirement): Promise<BrowserAudioWindowReader> {
    const digest = contentDigestHex(source.digest, "audio resource digest");
    let reader = this.audioReaders.get(digest);
    if (!reader) {
      const asset = this.assetByDigest.get(digest);
      if (!asset) throw new Error(`compiled audio resource ${source.digest} has no browser source`);
      const bytes = await this.fetchAssetBytes(asset, digest);
      reader = await BrowserAudioWindowReader.open({ blob: new Blob([Uint8Array.from(bytes).buffer]) });
      this.audioReaders.set(digest, reader);
    }
    if (reader.channels !== source.sourceChannels) throw new Error(`decoded audio has ${reader.channels} channels; expected ${source.sourceChannels}`);
    this.audioReaders.delete(digest); this.audioReaders.set(digest, reader);
    return reader;
  }

  private async audioWindow(context: BaseAudioContext, source: AudioSourceRequirement, reader: BrowserAudioWindowReader): Promise<AudioBuffer> {
    const key = `${source.digest}@${context.sampleRate}/${source.startSample}`;
    let buffer = this.audioBuffers.get(key);
    if (!buffer) {
      const end = Math.min(source.startSample + context.sampleRate, Math.round(reader.duration * context.sampleRate));
      if (end < source.endSample) throw new Error("compiled source sample is outside decoded PCM");
      buffer = await reader.read(context, source.startSample, end);
      this.audioBuffers.set(key, buffer);
    }
    this.audioBuffers.delete(key); this.audioBuffers.set(key, buffer);
    const cacheBytes = Math.min(4 * 1024 ** 2, this.runtimeBudget.memoryBytes / 16);
    while (this.audioCacheBytes() > cacheBytes && this.audioBuffers.size) this.audioBuffers.delete(this.audioBuffers.keys().next().value!);
    return buffer;
  }

  private audioCacheBytes(): number {
    let bytes = 0; for (const buffer of this.audioBuffers.values()) bytes += buffer.length * buffer.numberOfChannels * 4;
    return bytes;
  }

  private renderCompiledAudioBuffer(context: BaseAudioContext, start: number, end: number): Promise<AudioBuffer> {
    const work = this.audioTail.catch(() => undefined).then(() => this.renderCompiledAudioBufferNow(context, start, end));
    this.audioTail = work;
    return work;
  }

  private async renderCompiledAudioBufferNow(
    context: BaseAudioContext,
    startSample: number,
    endSample: number,
  ): Promise<AudioBuffer> {
    const program = this.requireCompiledAudioProgram();
    if (context.sampleRate !== program.sampleRate) {
      throw new Error(
        `audio render context is ${context.sampleRate} Hz; active render requires ${program.sampleRate} Hz`,
      );
    }
    if (
      !Number.isSafeInteger(startSample) ||
      !Number.isSafeInteger(endSample) ||
      startSample < 0 ||
      endSample <= startSample ||
      endSample > program.sampleCount
    ) {
      throw new RangeError(
        `invalid compiled audio range [${startSample}, ${endSample})`,
      );
    }
    const engine = this.engine;
    const renderId = this.renderId;
    const output = context.createBuffer(
      2,
      endSample - startSample,
      program.sampleRate,
    );
    const leftOutput = output.getChannelData(0);
    const rightOutput = output.getChannelData(1);
    const blockSize = 8_192;
    for (
      let blockStart = startSample;
      blockStart < endSample;
      blockStart += blockSize
    ) {
      const blockEnd = Math.min(endSample, blockStart + blockSize);
      const priority: RuntimePriority = this.runtimePurpose === "export" ? "export" : "playback";
      const release = await this.runtimeBudget.acquirePresentation(priority, this.runtimeAbort.signal);
      let finishMemory: ((retainedBytes: number) => void) | undefined;
      let mixed: Float32Array;
      try {
        this.runtimeAbort.signal.throwIfAborted();
        const requirements = parseAudioSources(JSON.parse(await this.runtimeCall<string>("audio_windows_json",
          renderId, BigInt(blockStart), BigInt(blockEnd))));
        const readers = [];
        for (const source of requirements) readers.push({ source, reader: await this.audioReaderForDigest(source) });
        const workingBytes = readers.reduce((sum, entry) => sum + entry.reader.workingBytes(context.sampleRate),
          (blockEnd - blockStart) * 24 + 1024 * 1024);
        finishMemory = await this.runtimeBudget.admitFrameAfterKernels(this.audioMemoryOwner, workingBytes, this.runtimeAbort.signal);
        const windows = [];
        for (const { source, reader } of readers) {
          const buffer = await this.audioWindow(context, source, reader);
          windows.push({ digest: source.digest, startSample: source.startSample, left: buffer.getChannelData(0),
            right: source.sourceChannels === 2 ? buffer.getChannelData(1) : new Float32Array(0) });
        }
        if (this.closed || this.engine !== engine || this.renderId !== renderId) throw new Error("audio render was superseded by a different render package");
        mixed = await this.runtimeCall<Float32Array>("mix_audio_windows", renderId, BigInt(blockStart), BigInt(blockEnd), windows);
        const active = new Set(requirements.map(source => contentDigestHex(source.digest, "audio digest")));
        for (const [digest, reader] of this.audioReaders) if (!active.has(digest)) { reader.close(); this.audioReaders.delete(digest); }
      } finally {
        try { finishMemory?.(this.audioCacheBytes()); }
        finally { release(); }
      }
      if (
        !(mixed instanceof Float32Array) ||
        mixed.length !== (blockEnd - blockStart) * 2
      ) {
        throw new Error(
          "compiled audio PCM does not match the requested sample range",
        );
      }
      const offset = blockStart - startSample;
      for (let index = 0; index < blockEnd - blockStart; index += 1) {
        leftOutput[offset + index] = mixed[index * 2]!;
        rightOutput[offset + index] = mixed[index * 2 + 1]!;
      }
    }
    return output;
  }

  private scheduleAudioGraph(
    context: BaseAudioContext,
    playback: CompiledAudioPlayback,
    _mediaTime: number,
    destination: AudioNode = context.destination,
  ): AudioGraph {
    const master = context.createGain();
    master.gain.value = this.muted ? 0 : this.volume;
    master.connect(destination);
    const now = context.currentTime;
    const source = context.createBufferSource();
    source.buffer = playback.buffer;
    source.connect(master);
    source.start(now);
    const sources = [source];
    source.onended = () => { source.disconnect(); source.buffer = null; const index = sources.indexOf(source); if (index >= 0) sources.splice(index, 1); };
    this.stats.audioScheduledSources += sources.length;
    return {
      master,
      sources,
      nextSample: playback.startSample + playback.buffer.length,
      scheduledUntilContextTime: now + playback.buffer.duration,
    };
  }

  private async runAudioPump(
    generation: number,
    context: AudioContext,
    graph: AudioGraph,
  ): Promise<void> {
    const program = this.requireCompiledAudioProgram();
    while (
      this.playing &&
      !this.closed &&
      generation === this.playGeneration &&
      this.audioGraph === graph &&
      graph.nextSample < program.sampleCount
    ) {
      if (graph.scheduledUntilContextTime - context.currentTime > 0.5) {
        await waitMilliseconds(50);
        continue;
      }
      const startSample = graph.nextSample;
      const endSample = Math.min(
        program.sampleCount,
        startSample + program.sampleRate,
      );
      const buffer = await this.renderCompiledAudioBuffer(
        context,
        startSample,
        endSample,
      );
      if (
        !this.playing ||
        this.closed ||
        generation !== this.playGeneration ||
        this.audioGraph !== graph
      )
        return;
      const source = context.createBufferSource();
      source.buffer = buffer;
      source.connect(graph.master);
      const when = Math.max(
        context.currentTime,
        graph.scheduledUntilContextTime,
      );
      source.start(when);
      graph.sources.push(source);
      source.onended = () => { source.disconnect(); source.buffer = null; const index = graph.sources.indexOf(source); if (index >= 0) graph.sources.splice(index, 1); };
      graph.nextSample = endSample;
      graph.scheduledUntilContextTime = when + buffer.duration;
      this.stats.audioScheduledSources += 1;
    }
  }

  private teardownAudioGraph(): void {
    if (!this.audioGraph) return;
    for (const source of this.audioGraph.sources) {
      try {
        source.stop();
      } catch {
        /* already ended */
      }
      source.disconnect();
    }
    this.audioGraph.master.disconnect();
    this.audioGraph = null;
  }

  private applyMasterVolume(): void {
    if (this.audioGraph)
      this.audioGraph.master.gain.value = this.muted ? 0 : this.volume;
  }

  private async fetchAssetBytes(
    asset: AdmittedBrowserAsset,
    expectedDigest?: string,
  ): Promise<Uint8Array> {
    const url = this.assetUrl(asset);
    const expected =
      expectedDigest ??
      contentDigestHex(asset.contentDigest, `asset '${asset.id}' digest`);
    const key = `${expected}:${url}`;
    let promise = this.assetBytes.get(key);
    if (!promise) {
      promise = fetchBytes(url);
      this.assetBytes.set(key, promise);
      void promise.catch(() => {
        if (this.assetBytes.get(key) === promise) this.assetBytes.delete(key);
      });
      // Keep small shared resources, but do not retain every long encoded audio source.
      void promise.then(
        (bytes) => {
          if (
            bytes.byteLength > 32 * 1024 * 1024 &&
            this.assetBytes.get(key) === promise
          )
            this.assetBytes.delete(key);
        },
        () => {},
      );
    }
    return promise;
  }

  private assetUrl(asset: AdmittedBrowserAsset): string {
    if (/^https?:\/\//.test(asset.url) && this.proxyBase) {
      return `${this.proxyBase}?url=${encodeURIComponent(asset.url)}`;
    }
    return this.resolveAssetUrl(asset.url);
  }

  private resolveAssetUrl(value: string): string {
    return resolveAssetUrl(value, this.assetBaseUrl, null);
  }

  private requireAsset(id: string, type?: string): AdmittedBrowserAsset {
    const asset = this.assetsById.get(id);
    if (!asset) throw new Error(`asset '${id}' is not registered`);
    if (type && asset.type !== type)
      throw new Error(`asset '${id}' is not ${type}`);
    return asset;
  }

  private requireRenderReceipt(): ProductRenderReceipt {
    if (!this.renderReceipt)
      throw new Error("browser player has no open fixed render");
    return this.renderReceipt;
  }

  private requireCompiledAudioProgram(): CompiledAudioProgramWire {
    if (!this.compiledAudioProgram)
      throw new Error("browser player has no compiled audio program");
    return this.compiledAudioProgram;
  }

  private frameAtSeconds(seconds: number): number {
    if (!this.renderId) throw new Error("browser player has no open RenderId");
    return exactBigIntNumber(
      this.engine.quantize_frame_seconds(seconds, ...this.requireRenderReceipt().frameRate.split("/").map(Number) as [number, number], BigInt(this.requireRenderReceipt().frameCount)),
      "compiled frame clock",
    );
  }

  private sampleAtSeconds(seconds: number, allowEnd = false): number {
    const program = this.requireCompiledAudioProgram();
    if (allowEnd && seconds >= this.durationS()) return program.sampleCount;
    if (!this.renderId) throw new Error("browser player has no open RenderId");
    return exactBigIntNumber(
      this.engine.quantize_sample_seconds(seconds, program.sampleRate, BigInt(program.sampleCount)),
      "compiled sample clock",
    );
  }

  private allocateGeneration(): bigint {
    const value = this.generation;
    this.generation += 1n;
    if (value <= 0n) throw new Error("external generation id space exhausted");
    return value;
  }

  private framePlanningInput(
    frame: number,
    width: number,
    height: number,
    transparent: boolean,
  ): ProductFramePlanningInput {
    // Every surface borrows from the same frame/host allowance. An arbitrary quarter-frame
    // ceiling must not reject a large offscreen input when the complete live plan fits.
    const maxFrameBytes = BigInt(this.runtimeBudget.memoryBytes);
    const maxSurfaceBytes = maxFrameBytes;
    return {
      frame,
      width,
      height,
      transparent,
      maxSurfaceBytes,
      maxFrameBytes,
    };
  }

  private prefetchPlanningWindow(
    currentFrame: number, demandFirst=false,
  ): ProductFramePlanningJob[] {
    const planner = this.requirePlanner();
    const receipt = this.requireRenderReceipt();
    const lastFrame = receipt.frameCount - 1;
    const frame = Math.max(0, Math.min(lastFrame, Math.trunc(currentFrame)));
    // Keep two seconds of pure planning in flight. Packets are independently capped by the
    // planner's frame/byte limits, while the deeper queue prevents fast WASM workers from going
    // idle behind one-at-a-time playback advancement.
    const lookahead = Math.max(
      1,
      Math.ceil(canonicalRationalNumber(receipt.frameRate, "frameRate") * 2),
    );
    planner.retainPlaybackWindow(frame, lastFrame, lookahead);
    const canvasWidth = receipt.canvasWidth;
    const canvasHeight = receipt.canvasHeight;
    const width = Math.max(1, Math.round(canvasWidth * this.pxScale));
    const height = Math.max(1, Math.round(canvasHeight * this.pxScale));
    const firstOffset = this.playbackFrame === frame ? 1 : 0;
    const jobs: ProductFramePlanningJob[] = [];
    for (let offset = firstOffset; offset <= lookahead; offset += 1) {
      const candidate = frame + offset;
      if (candidate > lastFrame) break;
      jobs.push(
        demandFirst && offset===firstOffset
          ? planner.prepare(this.framePlanningInput(candidate,width,height,false),0)
          : planner.prefetch(this.framePlanningInput(candidate,width,height,false),offset),
      );
    }
    this.syncPlannerStats();
    return jobs;
  }

  private requirePlanner(): ProductFramePlannerPool {
    if (!this.planner)
      throw new Error("Product frame planner is not initialized");
    return this.planner;
  }

  private assertActiveRenderId(expected: string, received: string): void {
    if (received !== expected || this.renderId !== expected) {
      throw new Error(
        `Product render superseded: expected ${expected}, received ${received}, active ${this.renderId}`,
      );
    }
  }

  private syncPlannerStats(): void {
    const snapshot = this.planner?.snapshot();
    if (!snapshot) {
      this.stats.framePlannerWorkers = 0;
      this.stats.framePlannerQueueDepth = 0;
      this.stats.framePlannerReadyFrames = 0;
      this.stats.framePlannerReadyBytes = 0;
      return;
    }
    this.stats.framePlannerWorkers = snapshot.workerCount;
    this.stats.framePlannerReadyHits = snapshot.readyHits;
    this.stats.framePlannerWaitHits = snapshot.waitHits;
    this.stats.framePlannerQueued = snapshot.queued;
    this.stats.framePlannerCompleted = snapshot.completed;
    this.stats.framePlannerCancelled = snapshot.cancelled;
    this.stats.framePlannerErrors = snapshot.errors;
    this.stats.framePlannerQueueDepth = snapshot.queueDepth;
    this.stats.framePlannerReadyFrames = snapshot.readyFrames;
    this.stats.framePlannerReadyBytes = snapshot.readyBytes;
  }

  private async pickScene3d(
    canvasX: number,
    canvasY: number,
    clipId?: string,
  ): Promise<{ semanticAddress: string; nodeId: number } | null> {
    if (!Number.isFinite(canvasX) || !Number.isFinite(canvasY)) {
      throw new Error("Scene3D pick coordinates must be finite");
    }
    const devicePoint: [number, number] = [
      canvasX * this.inspectionScaleX,
      canvasY * this.inspectionScaleY,
    ];
    for (const placement of [...this.frameInspection.scene3d].reverse()) {
      if (clipId && placement.clipId !== clipId) continue;
      const inverse = inverseHomography(placement.deviceFromLocal);
      if (!inverse) continue;
      const local = applyHomography(inverse, devicePoint);
      if (!local) continue;
      const { x, y, width, height } = placement.bounds;
      if (
        width <= 0 ||
        height <= 0 ||
        local[0] < x ||
        local[1] < y ||
        local[0] >= x + width ||
        local[1] >= y + height
      )
        continue;
      const gpuRequest = this.currentScene3dRequests.get(placement.contentHash);
      if (!gpuRequest) continue;
      const frameWidth =
        gpuRequest?.width ??
        JSON.parse(new TextDecoder().decode(gpuRequest!.canonical)).width;
      const frameHeight =
        gpuRequest?.height ??
        JSON.parse(new TextDecoder().decode(gpuRequest!.canonical)).height;
      const pixelX = Math.min(
        frameWidth - 1,
        Math.floor(((local[0] - x) / width) * frameWidth),
      );
      const pixelY = Math.min(
        frameHeight - 1,
        Math.floor(((local[1] - y) / height) * frameHeight),
      );
      const memory = await this.runtimeBudget.acquireKernel(frameWidth * frameHeight * 128 + 8 * 1024 ** 2, this.runtimeAbort.signal);
      let pick: { semanticAddress: string; nodeId: number } | null;
      try {
        await this.ensureScene3dResources(gpuRequest.canonical);
        pick = JSON.parse(await this.runtimeCall<string>("pick_scene3d_request",
          placement.contentHash, gpuRequest.topology, gpuRequest.canonical, pixelX, pixelY));
      } finally { memory.release(); }
      if (pick) return pick;
    }
    return null;
  }
}

function admitBrowserAssets(
  assets: readonly BrowserResourceLocator[],
  manifest: ResourceManifest,
): AdmittedBrowserAssets {
  const byId = new Map<string, AdmittedBrowserAsset>();
  const byDigest = new Map<string, AdmittedBrowserAsset>();
  for (const asset of assets) {
    if (byId.has(asset.id))
      throw new Error(`asset '${asset.id}' is duplicated`);
    const entry = manifest.entries[asset.id];
    if (!entry) {
      throw new Error(
        `browser locator '${asset.id}' is absent from the pinned ResourceManifest`,
      );
    }
    const digest = contentDigestHex(
      entry.digest,
      `resource '${asset.id}' digest`,
    );
    const admitted: AdmittedBrowserAsset = {
      ...asset,
      type: entry.kind,
      contentDigest: entry.digest,
      descriptor: entry.descriptor,
    };
    byId.set(asset.id, admitted);
    if (!byDigest.has(digest)) byDigest.set(digest, admitted);
  }
  return { byId, byDigest };
}

function indexCompiledExecutionResources(snapshot: ProductRuntimeSnapshot): FrozenBundleResources {
  const fonts = new Map<string, Uint8Array>();
  const models = new Map<string, Uint8Array>();
  const environments = new Map<string, Uint8Array>();
  const shaders = new Map<string, Uint8Array>();
  const resources = snapshot.resources;
  if (!Array.isArray(resources)) {
    throw new Error("compiled execution resource index must be an array");
  }
  for (const rawResource of resources) {
    const resource = record(rawResource, "compiled execution resource");
    if (
      typeof resource.resourceId !== "string" ||
      resource.resourceId.length === 0
    ) {
      throw new Error("compiled execution resource id must be non-empty");
    }
    if (
      typeof resource.contentDigest !== "string" ||
      !/^sha256:[0-9a-f]{64}$/.test(resource.contentDigest)
    ) {
      throw new Error(
        `compiled execution resource '${resource.resourceId}' has an invalid digest`,
      );
    }
    if (
      resource.kind !== "font-bytes" &&
      resource.kind !== "model3d-bytes" &&
      resource.kind !== "environment-bytes" &&
      resource.kind !== "runtime-shader"
    ) {
      throw new Error(
        `compiled execution resource '${resource.resourceId}' has unknown kind '${String(resource.kind)}'`,
      );
    }
    if (resource.kind === "runtime-shader") {
      if (
        typeof resource.abiDigest !== "string" ||
        !/^sha256:[0-9a-f]{64}$/.test(resource.abiDigest)
      ) {
        throw new Error(
          `compiled runtime shader '${resource.resourceId}' has an invalid ABI digest`,
        );
      }
    } else if (resource.abiDigest != null) {
      throw new Error(
        `compiled resource '${resource.resourceId}' unexpectedly declares an ABI digest`,
      );
    }
    const bytes = resource.bytes;
    if (!(bytes instanceof Uint8Array)) throw new Error("compiled resource bytes are missing");
    const content = contentDigestHex(
      resource.contentDigest,
      `compiled resource '${resource.resourceId}' content digest`,
    );
    if (resource.kind === "font-bytes") {
      fonts.set(content, bytes);
    } else if (resource.kind === "environment-bytes") {
      environments.set(content, bytes);
    } else if (resource.kind === "model3d-bytes") {
      models.set(content, bytes);
    } else if (resource.kind === "runtime-shader") {
      const abi = contentDigestHex(
        resource.abiDigest,
        `compiled resource '${resource.resourceId}' ABI digest`,
      );
      shaders.set(`${content}:${abi}`, bytes);
    }
  }
  return { fonts, models, shaders, environments };
}

function replaceMap<K, V>(target: Map<K, V>, source: ReadonlyMap<K, V>): void {
  target.clear();
  for (const [key, value] of source) target.set(key, value);
}

function emptyPlayerStats(): PlayerStats {
  return {
    rAFFrames: 0,
    renders: 0,
    scrubs: 0,
    fontsRegistered: 0,
    videoFrames: 0,
    gpuVideoFrames: 0,
    clockMode: "performance",
    surfaceMode: "none",
    gpuFrames: 0,
    cpuFrames: 0,
    audioMode: "none",
    audioScheduledSources: 0,
    audioDecodeErrors: 0,
    audioPeaksDecodes: 0,
    preparedFrames: 0,
    perfFrameMs: 0,
    perfEvaluatePrepareMs: 0,
    perfRequestInspectMs: 0,
    perfResourceFulfillMs: 0,
    perfLowerMs: 0,
    perfBindPacketsMs: 0,
    perfExecuteMs: 0,
    perfExecutorPacketAdmissionMs: 0,
    perfExecutorProgramAdmissionMs: 0,
    perfExecutorScheduleAdmissionMs: 0,
    perfExecutorRenderMs: 0,
    programLocalPasses: 0,
    directRasterPrograms: 0,
    programGroups: 0,
    programTransformGroups: 0,
    programClipGroups: 0,
    pathClipElisions: 0,
    programOpacityGroups: 0,
    programFilterGroups: 0,
    programMaskGroups: 0,
    programShaderGroups: 0,
    programBackdropGroups: 0,
    programBlendGroups: 0,
    perfPresentMs: 0,
    perfCaptureEncodeMs: 0,
    perfVideoDecodeMs: 0,
    perfVideoCopyMs: 0,
    perfVideoWrapMs: 0,
    videoThumbExtracts: 0,
    planBytes: 0,
    bindingBytes: 0,
    scheduleBytes: 0,
    requestBytes: 0,
    executionPasses: 0,
    physicalSurfaces: 0,
    maximumLiveImages: 0,
    surfaceAllocations: 0,
    surfaceReuses: 0,
    surfaceAllocatedBytes: 0,
    maximumSurfaceResidentBytes: 0,
    maximumSurfacePeakBytes: 0,
    surfacePoolEvictions: 0,
    templateCacheHits: 0,
    templateCacheMisses: 0,
    programCacheHits: 0,
    programCacheMisses: 0,
    fontCacheHits: 0,
    fontCacheMisses: 0,
    shaderCacheHits: 0,
    shaderCacheMisses: 0,
    rasterLayerCacheHits: 0,
    rasterLayerCacheMisses: 0,
    maximumRasterLayerCacheEntries: 0,
    maximumRasterLayerCacheBytes: 0,
    resourceCacheGenerationInvalidations: 0,
    resourceCacheHits: 0,
    resourceCacheMisses: 0,
    resourceCacheInsertions: 0,
    resourceCacheEvictions: 0,
    resourceCacheBypasses: 0,
    maximumResourceCacheEntries: 0,
    maximumResourceCacheBytes: 0,
    renderId: "",
    framePlannerWorkers: 0,
    framePlannerReadyHits: 0,
    framePlannerWaitHits: 0,
    framePlannerQueued: 0,
    framePlannerCompleted: 0,
    framePlannerCancelled: 0,
    framePlannerErrors: 0,
    framePlannerQueueDepth: 0,
    framePlannerReadyFrames: 0,
    framePlannerReadyBytes: 0,
    perfFramePlannerWorkerMs: 0,
    perfFramePlannerTurnGapMs: 0,
    perfFramePlannerReleaseMs: 0,
    motionExecuteClips: {},
    motionPerformance: {},
    degradations: [],
  };
}

async function loadWasmModule(
  moduleUrl: string,
  wasmUrl: string,
): Promise<ValleWasmModule> {
  const module = (await import(moduleUrl)) as unknown as ValleWasmModule;
  await module.default(wasmUrl);
  return module;
}

async function loadCanvasKit(
  assets: PlayerRuntimeAssets["canvasKit"],
): Promise<CanvasKit> {
  canvasKitPromise ??= (async () => {
    const init =
      staticCanvasKitInit ?? (await injectCanvasKitScript(assets.glue));
    return init({
      locateFile: (file) =>
        file.endsWith(".wasm") ? assets.wasm : new URL(file, assets.glue).href,
    });
  })();
  return canvasKitPromise;
}

function injectCanvasKitScript(src: string): Promise<CanvasKitInitializer> {
  return new Promise((resolve, reject) => {
    if (typeof document === "undefined") {
      reject(new Error(`cannot load ${src}: no document`));
      return;
    }
    const script = document.createElement("script");
    script.src = src;
    script.onload = () =>
      typeof globalThis.CanvasKitInit === "function"
        ? resolve(globalThis.CanvasKitInit)
        : reject(new Error(`CanvasKitInit missing after loading ${src}`));
    script.onerror = () =>
      reject(new Error(`failed to load CanvasKit script ${src}`));
    document.head.appendChild(script);
  });
}

async function fetchBytes(url: string): Promise<Uint8Array> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`fetch ${url}: HTTP ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function videoFrameToRgba(
  frame: VideoFrame,
): Promise<{ width: number; height: number; data: Uint8Array }> {
  const width = frame.displayWidth;
  const height = frame.displayHeight;
  try {
    const data = new Uint8Array(width * height * 4);
    await frame.copyTo(data, { format: "RGBA", colorSpace: "srgb" });
    return { width, height, data };
  } catch {
    const canvas = new OffscreenCanvas(width, height);
    const context = canvas.getContext("2d");
    if (!context) throw new Error("OffscreenCanvas 2D context is unavailable");
    context.drawImage(frame, 0, 0, width, height);
    const image = context.getImageData(0, 0, width, height);
    return { width, height, data: new Uint8Array(image.data.buffer) };
  }
}

function makeLazyVideoFrameImage(
  CanvasKit: CanvasKit,
  frame: VideoFrame,
  info: PartialImageInfo,
): Image | null {
  const makeImage = CanvasKit.MakeLazyImageFromTextureSource as unknown as (
    source: VideoFrame,
    imageInfo?: PartialImageInfo,
  ) => Image | null;
  return makeImage.call(CanvasKit, frame, info);
}

function record(value: unknown, label: string): Wire {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value as Wire;
}

type ContentDigestWire = `sha256:${string}`;

function contentDigestWire(value: unknown, label: string): ContentDigestWire {
  if (typeof value !== "string" || !/^sha256:[0-9a-f]{64}$/.test(value)) {
    throw new Error(
      `${label} must be a sha256:<64 lowercase hex> ContentDigest`,
    );
  }
  return value as ContentDigestWire;
}

function contentDigestHex(value: unknown, label: string): string {
  return contentDigestWire(value, label).slice("sha256:".length);
}

function packedByteArray(value: unknown, label: string): Uint8Array {
  if (!Array.isArray(value)) throw new Error(`${label} must be a byte array`);
  return Uint8Array.from(
    value.map((item, index) => {
      const number = typeof item === "bigint" ? Number(item) : item;
      if (
        !Number.isInteger(number) ||
        Number(number) < 0 ||
        Number(number) > 255
      ) {
        throw new Error(`${label}[${index}] is not a byte`);
      }
      return Number(number);
    }),
  );
}

function inverseHomography(matrix: readonly number[]): number[] | null {
  if (matrix.length !== 9 || !matrix.every(Number.isFinite)) return null;
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
  const determinant =
    a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
  if (!Number.isFinite(determinant) || Math.abs(determinant) <= Number.EPSILON)
    return null;
  const scale = 1 / determinant;
  const inverse = [
    (e * i - f * h) * scale,
    (c * h - b * i) * scale,
    (b * f - c * e) * scale,
    (f * g - d * i) * scale,
    (a * i - c * g) * scale,
    (c * d - a * f) * scale,
    (d * h - e * g) * scale,
    (b * g - a * h) * scale,
    (a * e - b * d) * scale,
  ];
  return inverse.every(Number.isFinite) ? inverse : null;
}

function applyHomography(
  matrix: readonly number[],
  point: readonly [number, number],
): [number, number] | null {
  const [x, y] = point;
  const w = matrix[6]! * x + matrix[7]! * y + matrix[8]!;
  if (!Number.isFinite(w) || Math.abs(w) <= Number.EPSILON) return null;
  const mapped: [number, number] = [
    (matrix[0]! * x + matrix[1]! * y + matrix[2]!) / w,
    (matrix[3]! * x + matrix[4]! * y + matrix[5]!) / w,
  ];
  return mapped.every(Number.isFinite) ? mapped : null;
}

function rationalSeconds(value: unknown): number {
  if (typeof value !== "string" || !/^-?(0|[1-9]\d*)\/[1-9]\d*$/.test(value))
    throw new Error("RationalTime must be an exact num/den string");
  const [numerator, denominator] = value.split("/").map(Number);
  if (
    !Number.isSafeInteger(numerator) ||
    !Number.isSafeInteger(denominator) ||
    denominator! <= 0
  )
    throw new Error("RationalTime is invalid");
  return numerator! / denominator!;
}

function exactInteger(value: unknown, label: string): number {
  const number = typeof value === "bigint" ? Number(value) : value;
  if (!Number.isSafeInteger(number) || Number(number) <= 0)
    throw new Error(`${label} must be positive`);
  return Number(number);
}

function pixelBytes(
  width: unknown,
  height: unknown,
  bytesPerPixel: number,
): number {
  const w = exactInteger(width, "resource width");
  const h = exactInteger(height, "resource height");
  const bytes = w * h * bytesPerPixel;
  if (!Number.isSafeInteger(bytes))
    throw new Error("resource byte size overflowed");
  return bytes;
}

function noop(): void {}

function disposeAll(disposers: Iterable<() => void>): void {
  let firstError: unknown = null;
  for (const dispose of disposers) {
    try {
      dispose();
    } catch (error) {
      firstError ??= error;
    }
  }
  if (firstError !== null) throw firstError;
}

function positiveInteger(value: unknown, label: string): number {
  if (!Number.isInteger(value) || Number(value) <= 0)
    throw new Error(`${label} must be positive`);
  return Number(value);
}

function finitePositive(value: unknown, label: string): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    throw new Error(`${label} must be finite and positive`);
  }
  return value;
}

function clamp(value: number, low: number, high: number): number {
  return Math.max(low, Math.min(high, Number.isFinite(value) ? value : low));
}

function maxBigInt(left: bigint, right: bigint): bigint {
  return left > right ? left : right;
}

function exactBigIntNumber(value: bigint, label: string): number {
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < 0)
    throw new Error(`${label} exceeds the exact reporting range`);
  return number;
}

function maybeCreateAudioContext(sampleRate: number): AudioContext | null {
  const Constructor = globalThis.AudioContext ?? globalThis.webkitAudioContext;
  try {
    return Constructor ? new Constructor({ sampleRate }) : null;
  } catch {
    return null;
  }
}

function waitMilliseconds(milliseconds: number): Promise<void> {
  return new Promise((resolve) =>
    setTimeout(resolve, Math.max(0, milliseconds)),
  );
}

/** Let worker replies and UI events run without accumulating nested timer clamps. */
function yieldToHost(): Promise<void> {
  const scheduler = (globalThis as unknown as { scheduler?: { yield(): Promise<void> } }).scheduler;
  if (scheduler?.yield) return scheduler.yield();
  if (typeof MessageChannel === "undefined") return waitMilliseconds(0);
  const channel = new MessageChannel();
  return new Promise(resolve => {
    channel.port1.onmessage = () => {
      channel.port1.close(); channel.port2.close(); resolve();
    };
    channel.port2.postMessage(null);
  });
}

function canonicalRationalNumber(value: string, label: string): number {
  const match = /^(-?(?:0|[1-9][0-9]*))\/([1-9][0-9]*)$/.exec(value);
  if (!match) throw new TypeError(`${label} must be a canonical rational`);
  const numerator = Number(match[1]);
  const denominator = Number(match[2]);
  const result = numerator / denominator;
  if (!Number.isFinite(result))
    throw new TypeError(`${label} exceeds the Web numeric range`);
  return result;
}

function parseCompiledAudioProgram(value: unknown): CompiledAudioProgramWire {
  const wire = record(value, "compiled audio program");
  if (!Array.isArray(wire.tracks)) {
    throw new TypeError("compiled audio program must contain tracks");
  }
  for (const field of ["sampleRate", "sampleCount"] as const) {
    if (!Number.isSafeInteger(wire[field]) || Number(wire[field]) <= 0) {
      throw new TypeError(
        `compiled audio program ${field} must be a positive safe integer`,
      );
    }
  }
  return wire as CompiledAudioProgramWire;
}

function parseAudioSources(value: unknown): AudioSourceRequirement[] {
  if (!Array.isArray(value))
    throw new TypeError("compiled audio sources must be an array");
  return value.map((entry) => {
    const source = record(entry, "compiled audio source");
    if (
      !Number.isSafeInteger(source.startSample) || source.startSample < 0 ||
      !Number.isSafeInteger(source.endSample) || source.endSample <= source.startSample ||
      (source.sourceChannels !== 1 && source.sourceChannels !== 2)
    ) {
      throw new TypeError("compiled audio source has invalid index/channels");
    }
    return {
      startSample: source.startSample,
      endSample: source.endSample,
      digest: contentDigestWire(source.digest, "compiled audio source digest"),
      sourceChannels: source.sourceChannels,
    };
  });
}

class AudioMasterClock {
  mode = "performance";
  private mediaTime = 0;
  private startedAt = 0;
  constructor(private readonly context: AudioContext | null) {}

  async start(
    mediaTime: number,
    { useAudioClock }: { useAudioClock: boolean },
  ): Promise<void> {
    this.mediaTime = mediaTime;
    if (useAudioClock && this.context) {
      if (this.context.state === "suspended") await this.context.resume();
      this.startedAt = this.context.currentTime;
      this.mode = "audio";
    } else {
      this.startedAt = performance.now() / 1000;
      this.mode = "performance";
    }
  }

  now(): number {
    const clock =
      this.mode === "audio" && this.context
        ? this.context.currentTime
        : performance.now() / 1000;
    return this.mediaTime + Math.max(0, clock - this.startedAt);
  }

  pause(mediaTime: number): void {
    this.mediaTime = mediaTime;
  }
}

declare global {
  var CanvasKitInit: CanvasKitInitializer | undefined;
  var webkitAudioContext: typeof AudioContext | undefined;
}
