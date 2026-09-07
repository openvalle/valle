import canvasKitPackage from "canvaskit-wasm/package.json";
// Browser Product Compositor host.
//
// Rust/WASM owns Timeline evaluation, Motion/caption preparation, RenderGraph construction and
// lowering. This file owns only browser resources, clock/scheduling, an opaque generation-scoped
// object table, CanvasKit execution and presentation.

import {
  RESOURCE_REQUESTS_ABI,
  CanvasKitExecutor,
  decodePackedAbi,
  type CanvasKitExecutionProfile,
  type CanvasKitExternalObject,
  type PackedValue,
} from "@valle/engine";
import type {
  ResourceManifest,
  TimelineDocument,
} from "@valle/engine/internal";
import { createDecoderRing, keyframeThumbnails, type DecoderRing } from "../media/decoder-ring.ts";
import { resolvePlayerRuntimeAssets, type PlayerRuntimeAssets } from "../runtime-assets.ts";
import { canonicalizeTimelineDocumentWithWasm } from "../timeline-compiler.ts";
import {
  ProductFramePlannerPool,
  type ProductFramePlanningInput,
  type ProductFramePlanningJob,
} from "../compositor/frame-pool.ts";
import {
  emptyProductEngineBootstrap,
  parseProductRenderReceipt,
  type ProductEngineBootstrap,
  type ProductEngineWire,
  type ProductRenderReceipt,
} from "../compositor/frame-protocol.ts";
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
  SkottieAnimation,
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
  canvas?: HTMLCanvasElement | null;
  assetBaseUrl?: string;
  proxyBase?: string | null;
  runtimeAssets: unknown;
  runtimeBaseUrl?: string | URL;
  pxScale?: number;
  gpu?: boolean;
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
  Partial<Pick<BrowserValleWebPlayerOptions, "assets">>;

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
    motionFrames?: {
      sourceDurationFrames: number;
      enterFrames: number | null;
      exitFrames: number | null;
      cues: Record<string,
        { type: "sourceRange"; startFrame: number; endFrame: number; enterFrames: number; exitFrames: number }
      >;
    };
  }>;
}

interface RenderOptions {
  png?: boolean;
  frame?: number;
  rasterScale?: number;
  transparent?: boolean;
}

interface BrowserProductEngineWire extends ProductEngineWire {
  scene3d_resource_needs_json(canonicalRequest: Uint8Array): string;
  register_scene3d_texture(
    digest: string,
    encodedBytes: Uint8Array,
    width: number,
    height: number,
    premulRgba8: Uint8Array,
  ): void;
  render_scene3d_request(
    contentDigest: string,
    topologyDigest: string,
    canonicalRequest: Uint8Array,
  ): Uint8Array;
  scene3d_frame_width(contentDigest: string): number;
  scene3d_frame_height(contentDigest: string): number;
  scene3d_pick_json(contentDigest: string, pixelX: number, pixelY: number): string;
  reset(): void;
}

interface ValleWasmModule {
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

export interface AudioBlockWire {
  renderId: string;
  sampleRate: number;
  startSample: number;
  endSample: number;
  samples: AudioSampleWire[];
}

export interface AudioSampleWire {
  renderId: string;
  sample: number;
  sampleTime: string;
  tracks: Array<{
    trackOrder: number;
    endpoints: AudioEndpointWire[];
  }>;
}

export interface AudioEndpointWire {
  sourceIndex: number;
  sourceSampleIndex: number;
  mappedTime:
    | { type: "static" }
    | { type: "exact"; time: string }
    | { type: "hold-start" }
    | { type: "hold-end" };
  digest: string | null;
  handle: number | null;
  decodedPcmDigest: string;
  sourceChannels: 1 | 2;
  crossfadeGain: number;
  gain: number;
  pan: number;
  leftGain: number;
  rightGain: number;
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

interface LottieRuntime {
  animation: SkottieAnimation;
  surface: Surface;
  width: number;
  height: number;
  fps: number;
}

interface FrozenBundleResources {
  fonts: Map<string, Uint8Array>;
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
  dispose: Array<() => void>;
  videoFrames: number;
  videoTextureFrames: number;
  lottieFrames: number;
  resourceCache: BrowserResourceCacheFrameReport;
}

interface CachedBrowserResource {
  object: CanvasKitExternalObject;
  videoFrames: number;
  videoTextureFrames: number;
  lottieFrames: number;
}

interface ProducedBrowserResource extends CachedBrowserResource {
  bytes: number;
  dispose: () => void;
}

export interface ProductRenderStats {
  videoFrames: number;
  gpuVideoFrames: number;
  lottieFrames: number;
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
    deviceFromLocal: [number, number, number, number, number, number, number, number, number];
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
  lottieFrames: number;
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
const staticCanvasKitInit = typeof globalThis.CanvasKitInit === "function"
  ? globalThis.CanvasKitInit
  : null;

export async function createBrowserValleWebPlayer(
  options: BrowserValleWebPlayerOptions,
): Promise<BrowserValleWebPlayer> {
  const player = new BrowserValleWebPlayer(options);
  await player.init();
  return player;
}

export function resolveAssetUrl(
  value: string,
  assetBaseUrl = "/assets/",
  proxyBase: string | null = null,
): string {
  if (/^https?:\/\//.test(value) && proxyBase) {
    return `${proxyBase}?url=${encodeURIComponent(value)}`;
  }
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(value) || value.startsWith("/")) return value;
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
  readonly stats: PlayerStats;
  hitRects: ProductHitRect[] = [];

  private readonly runtimeAssets: PlayerRuntimeAssets;
  private readonly wasmModuleUrl: string;
  private readonly wasmUrl: string;
  private wasm!: ValleWasmModule;
  private engine!: BrowserProductEngineWire;
  private planner: ProductFramePlannerPool | null = null;
  private bootstrap!: ProductEngineBootstrap;
  private CanvasKit!: CanvasKit;
  private compositor!: CanvasKitExecutor;
  private renderId = "";
  private generation = 1n;
  private timeS = 0;
  private playGeneration = 0;
  private rafId: number | null = null;
  private renderInFlight: Promise<void> | null = null;
  private playbackPump: Promise<void> | null = null;
  private audioPump: Promise<void> | null = null;
  private playbackFrame: number | null = null;
  private target: TargetSurface | null = null;
  private readonly assetsById = new Map<string, AdmittedBrowserAsset>();
  private readonly assetByDigest = new Map<string, AdmittedBrowserAsset>();
  private readonly assetBytes = new Map<string, Promise<Uint8Array>>();
  private readonly scene3dResourceTasks = new Map<string, Promise<void>>();
  private readonly videoRings = new Map<string, DecoderRing>();
  private readonly lottie = new Map<string, LottieRuntime>();
  private readonly fontBytes = new Map<string, Uint8Array>();
  private readonly shaderBytes = new Map<string, Uint8Array>();
  private readonly resourceObjects: BrowserResourceCache<CachedBrowserResource>;
  private renderReceipt: ProductRenderReceipt | null = null;
  private compiledAudioProgram: CompiledAudioProgramWire | null = null;
  private audioBuffers = new Map<string, Promise<AudioBuffer>>();
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
    if (!options?.fixedPackageManifestJson) throw new Error("fixedPackageManifestJson is required");
    if (!options?.timelineJson) throw new Error("timelineJson is required");
    if (!options.resourceManifestJson) throw new Error("resourceManifestJson is required");
    if (!options.verifiedBindingBundleJson) throw new Error("verifiedBindingBundleJson is required");
    this.runtimeAssets = resolvePlayerRuntimeAssets(options.runtimeAssets, options.runtimeBaseUrl);
    this.wasmModuleUrl = this.runtimeAssets.engine.glue;
    this.wasmUrl = this.runtimeAssets.engine.wasm;
    this.fixedPackageManifestJson = options.fixedPackageManifestJson;
    this.timelineJson = options.timelineJson;
    this.timeline = JSON.parse(options.timelineJson) as TimelineDocument;
    this.resourceManifestJson = options.resourceManifestJson;
    this.resourceManifest = JSON.parse(options.resourceManifestJson) as ResourceManifest;
    this.verifiedBindingBundleJson = options.verifiedBindingBundleJson;
    this.assets = options.assets ?? [];
    this.canvas = options.canvas ?? null;
    this.assetBaseUrl = options.assetBaseUrl ?? "/assets/";
    this.proxyBase = options.proxyBase ?? null;
    this.pxScale = finitePositive(options.pxScale ?? 1, "pxScale");
    this.gpu = options.gpu ?? true;
    this.audioContext = options.audioContext
      ?? maybeCreateAudioContext(this.timeline.document.canvas.sampleRate);
    this.resourceObjects = new BrowserResourceCache(options.resourceCache);
    this.clock = new AudioMasterClock(this.audioContext);
    this.stats = emptyPlayerStats();
  }

  async init(): Promise<this> {
    [this.wasm, this.CanvasKit] = await Promise.all([
      loadWasmModule(this.wasmModuleUrl, this.wasmUrl),
      loadCanvasKit(this.runtimeAssets.canvasKit),
    ]).then(([wasm, canvasKit]) => [wasm, canvasKit]);
    const staged = await this.stageRenderPackage(this.currentRenderPackage());
    this.commitStagedRenderPackage(staged);
    return this;
  }

  durationS(): number {
    const receipt = this.requireRenderReceipt();
    return receipt.sampleCount / receipt.sampleRate;
  }

  lastFrameTimeS(): number {
    const receipt = this.requireRenderReceipt();
    return (receipt.frameCount - 1) / canonicalRationalNumber(receipt.frameRate, "frameRate");
  }

  currentTime(): number {
    return this.timeS;
  }

  async seek(timeS: number, options: RenderOptions = {}) {
    this.pause();
    if (this.renderInFlight) await this.renderInFlight.catch(() => undefined);
    this.requirePlanner().advanceEpoch();
    this.timeS = clamp(timeS, 0, this.lastFrameTimeS());
    const rendered = await this.renderTime(this.timeS, options);
    this.playbackFrame = rendered.frame.index;
    this.onTimeUpdate?.(this.timeS);
    return rendered;
  }

  async scrub(timeS: number) {
    this.stats.scrubs += 1;
    return this.seek(timeS, { png: true });
  }

  async renderTime(timeS: number, options: RenderOptions = {}) {
    const frame = options.frame ?? this.frameAtSeconds(clamp(timeS, 0, this.lastFrameTimeS()));
    return this.renderFrame(frame, options);
  }

  async renderFrame(frame: number, options: RenderOptions = {}) {
    if (this.closed) throw new Error("browser player is closed");
    const renderId = this.renderId;
    if (!renderId) throw new Error("browser player has no open RenderId");
    const receipt = this.requireRenderReceipt();
    const frameCount = receipt.frameCount;
    if (!Number.isSafeInteger(frame) || frame < 0 || frame >= frameCount) {
      throw new RangeError(`frame ${frame} is outside fixed render range [0, ${frameCount})`);
    }
    const canvasWidth = receipt.canvasWidth;
    const canvasHeight = receipt.canvasHeight;
    const scale = finitePositive(options.rasterScale ?? this.pxScale, "rasterScale");
    const width = Math.max(1, Math.round(canvasWidth * scale));
    const height = Math.max(1, Math.round(canvasHeight * scale));
    const forceCpu = options.png === true;
    const target = this.acquireTarget(width, height, forceCpu);
    const frameStarted = performance.now();
    const planningInput = this.framePlanningInput(frame, width, height, options.transparent === true);
    const planner = this.requirePlanner();
    const planning = planner.prepare(planningInput);
    let fulfilled: FulfilledFrame | null = null;
    try {
      const requestStage = await planning.requests;
      this.assertActiveRenderId(renderId, requestStage.renderId);
      const frameInspection = JSON.parse(requestStage.inspectionJson) as ProductFrameInspection;
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
      const fulfillmentStarted = performance.now();
      const fulfillment = this.fulfillRequests(requestStage.requestPacket, target.gpu);
      const [readyResult, fulfillmentResult] = await Promise.allSettled([
        planning.ready,
        fulfillment,
      ] as const);
      if (readyResult.status === "rejected") throw readyResult.reason;
      if (fulfillmentResult.status === "rejected") throw fulfillmentResult.reason;
      const readyStage = readyResult.value;
      this.assertActiveRenderId(renderId, readyStage.renderId);
      const frameResources = fulfillmentResult.value;
      fulfilled = frameResources;
      const resourceFulfillMs = performance.now() - fulfillmentStarted;
      const generation = readyStage.generation;
      const planPacket = readyStage.planPacket;
      const bindingPacket = readyStage.bindingPacket;
      const schedulePacket = readyStage.schedulePacket;
      this.stats.planBytes += readyStage.planPacketTransferred ? planPacket.byteLength : 0;
      this.stats.bindingBytes += bindingPacket.byteLength;
      this.stats.scheduleBytes += schedulePacket.byteLength;
      const executeStarted = performance.now();
      this.assertActiveRenderId(renderId, readyStage.renderId);
      const report = await this.compositor.execute(
        planPacket,
        bindingPacket,
        schedulePacket,
        { generation, objects: frameResources.objects },
        {
          surface: target.surface,
          directContext: target.context,
          maxSurfaceBytes: planningInput.maxSurfaceBytes,
          maxFrameBytes: planningInput.maxFrameBytes,
        },
      );
      this.assertActiveRenderId(renderId, readyStage.renderId);
      const executeMs = performance.now() - executeStarted;
      const presentStarted = performance.now();
      this.presentTarget(target, width, height);
      const presentMs = performance.now() - presentStarted;
      const captureEncodeStarted = performance.now();
      const png = options.png ? this.encodeTargetPng(target.surface) : null;
      const captureEncodeMs = performance.now() - captureEncodeStarted;
      const stats: ProductRenderStats = {
        videoFrames: frameResources.videoFrames,
        gpuVideoFrames: frameResources.videoTextureFrames,
        lottieFrames: frameResources.lottieFrames,
        passes: report.passes,
        programs: report.programs,
        physicalSurfaces: report.physicalSurfaces,
        maximumLiveImages: report.maximumLiveImages,
        surfaceAllocations: report.surfaceAllocations,
        surfaceReuses: report.surfaceReuses,
        surfaceAllocatedBytes: exactBigIntNumber(report.surfaceAllocatedBytes, "surface allocated bytes"),
        surfaceResidentBytes: exactBigIntNumber(report.surfaceResidentBytes, "surface resident bytes"),
        surfacePeakBytes: exactBigIntNumber(report.surfacePeakBytes, "surface peak bytes"),
        surfacePoolEvictions: report.surfacePoolEvictions,
        templateCacheHit: readyStage.templateCacheHit,
        programCacheHits: report.programCacheHits,
        programCacheMisses: report.programCacheMisses,
        fontCacheHits: report.fontCacheHits,
        fontCacheMisses: report.fontCacheMisses,
        shaderCacheHits: report.shaderCacheHits,
        shaderCacheMisses: report.shaderCacheMisses,
        resourceCacheGeneration: frameResources.resourceCache.generation,
        resourceCacheGenerationInvalidations: frameResources.resourceCache.generationInvalidations,
        resourceCacheHits: frameResources.resourceCache.hits,
        resourceCacheMisses: frameResources.resourceCache.misses,
        resourceCacheInsertions: frameResources.resourceCache.insertions,
        resourceCacheEvictions: frameResources.resourceCache.evictions,
        resourceCacheBypasses: frameResources.resourceCache.bypasses,
        resourceCacheResidentEntries: frameResources.resourceCache.residentEntries,
        resourceCacheResidentBytes: frameResources.resourceCache.residentBytes,
      };
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
      this.stats.lottieFrames += stats.lottieFrames;
      this.stats.executionPasses += report.passes;
      this.stats.physicalSurfaces = Math.max(this.stats.physicalSurfaces, report.physicalSurfaces);
      this.stats.maximumLiveImages = Math.max(this.stats.maximumLiveImages, report.maximumLiveImages);
      this.stats.surfaceAllocations += report.surfaceAllocations;
      this.stats.surfaceReuses += report.surfaceReuses;
      this.stats.surfaceAllocatedBytes += exactBigIntNumber(report.surfaceAllocatedBytes, "surface allocated bytes");
      this.stats.maximumSurfaceResidentBytes = Math.max(
        this.stats.maximumSurfaceResidentBytes,
        exactBigIntNumber(report.surfaceResidentBytes, "surface resident bytes"),
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
      this.stats.resourceCacheGenerationInvalidations +=
        frameResources.resourceCache.generationInvalidations;
      this.stats.resourceCacheHits += frameResources.resourceCache.hits;
      this.stats.resourceCacheMisses += frameResources.resourceCache.misses;
      this.stats.resourceCacheInsertions += frameResources.resourceCache.insertions;
      this.stats.resourceCacheEvictions += frameResources.resourceCache.evictions;
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
        const clip = this.stats.motionPerformance[motion.clipId] ??= {
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
        };
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
      this.lastRender = {
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
      this.frameInspection = frameInspection;
      this.hitRects = frameInspection.hitRects;
      this.inspectedFrame = frame;
      this.inspectionScaleX = width / canvasWidth;
      this.inspectionScaleY = height / canvasHeight;
      return this.lastRender;
    } finally {
      planner.release(planning.key);
      if (fulfilled) {
        try {
          this.resourceObjects.releaseFrame();
        } finally {
          disposeAll(fulfilled.dispose);
        }
      }
    }
  }

  async play(): Promise<void> {
    if (this.closed) throw new Error("browser player is closed");
    if (this.playing) return;
    if (this.timeS >= this.lastFrameTimeS()) this.timeS = 0;
    const generation = ++this.playGeneration;
    const planning = this.prefetchPlanningWindow(
      this.frameAtSeconds(this.timeS),
    );
    const audio = this.prefetchAudio(this.timeS, this.audioContext);
    // Start the media clock only after a bounded decode-independent cushion exists. Without this
    // phase the clock spends its first transition consuming the very frames the worker is still
    // producing, and a realtime-capable planner can never recover that initial debt.
    await Promise.all(planning.slice(0, 16).map((job) => job.ready));
    const prefetched = await audio;
    if (generation !== this.playGeneration || this.closed) return;
    await this.clock.start(this.timeS, { useAudioClock: prefetched !== null });
    if (generation !== this.playGeneration || this.closed) return;
    this.teardownAudioGraph();
    if (this.audioContext && prefetched) {
      this.audioGraph = this.scheduleAudioGraph(this.audioContext, prefetched, this.timeS);
      this.stats.audioMode = "webaudio";
    } else {
      this.stats.audioMode = "performance";
    }
    this.playing = true;
    this.stats.clockMode = this.clock.mode;
    this.scheduleTick();
    if (this.audioGraph && this.audioContext) {
      const audioPump = this.runAudioPump(generation, this.audioContext, this.audioGraph);
      this.audioPump = audioPump;
      audioPump.catch((error) => {
        if (generation !== this.playGeneration || this.closed) return;
        this.pause();
        queueMicrotask(() => { throw error; });
      }).finally(() => {
        if (this.audioPump === audioPump) this.audioPump = null;
      });
    }
    const pump = this.runPlaybackPump(generation);
    this.playbackPump = pump;
    pump.catch((error) => {
      if (generation !== this.playGeneration || this.closed) return;
      this.pause();
      queueMicrotask(() => { throw error; });
    }).finally(() => {
      if (this.playbackPump === pump) this.playbackPump = null;
    });
  }

  pause(): void {
    this.playGeneration += 1;
    if (this.rafId !== null) cancelAnimationFrame(this.rafId);
    this.rafId = null;
    if (this.playing) this.timeS = clamp(this.clock.now(), 0, this.lastFrameTimeS());
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
    const buffer = await this.renderCompiledAudioBuffer(offline, startSample, endSample);
    this.scheduleAudioGraph(offline, { buffer, startSample }, start, offline.destination);
    return offline.startRendering();
  }

  async replaceRenderPackage(next: RenderPackageReplacement) {
    if (this.closed) throw new Error("browser player is closed");
    if (!next?.timelineJson) throw new Error("replaceRenderPackage requires timelineJson");
    const resume = this.playing;
    this.pause();
    if (this.renderInFlight) await this.renderInFlight.catch(() => undefined);
    let staged: StagedRenderPackage;
    try {
      staged = await this.stageRenderPackage(next);
    } catch (error) {
      if (resume) {
        try { await this.play(); } catch { /* preserve the admission failure */ }
      }
      throw error;
    }
    this.commitStagedRenderPackage(staged);
    this.timeS = clamp(this.timeS, 0, this.lastFrameTimeS());
    const rendered = await this.seek(this.timeS);
    if (resume) await this.play();
    return rendered;
  }

  /** Validate one complete generated document with the same Rust constructor used by Project. */
  canonicalizeTimelineDocument(timeline: TimelineDocument): CanonicalTimelineDocument {
    if (!this.wasm) throw new Error("Timeline canonical validation requires a loaded player runtime");
    return canonicalizeTimelineDocumentWithWasm(this.wasm, timeline);
  }

  async videoKeyframeThumbnails(
    assetId: string,
    { height = 26, maxCount = 60 }: { height?: number; maxCount?: number } = {},
  ) {
    const asset = this.requireAsset(assetId, "video");
    const bytes = await this.fetchAssetBytes(asset);
    this.stats.videoThumbExtracts += 1;
    return keyframeThumbnails({ buffer: Uint8Array.from(bytes).buffer, height, maxCount });
  }

  async audioPeaks(assetId: string, { buckets = 2000 }: { buckets?: number } = {}) {
    const asset = this.requireAsset(assetId, "audio");
    const bytes = await this.fetchAssetBytes(asset);
    this.stats.audioPeaksDecodes += 1;
    const context = new OfflineAudioContext(1, 1, 8000);
    const decoded = await context.decodeAudioData(bytes.slice().buffer);
    const source = decoded.getChannelData(0);
    const peaks = new Float32Array(buckets);
    for (let bucket = 0; bucket < buckets; bucket += 1) {
      const start = Math.floor(bucket * source.length / buckets);
      const end = Math.floor((bucket + 1) * source.length / buckets);
      let peak = 0;
      for (let index = start; index < end; index += 1) peak = Math.max(peak, Math.abs(source[index]!));
      peaks[bucket] = peak;
    }
    return { peaks, durationS: decoded.duration };
  }

  async motionInspection(
    clipId: string,
    canvasX: number,
    canvasY: number,
  ): Promise<{
    object: { semanticAddress: string } | null;
    compositionFrame: number;
    sourceFrame: number;
    boxes: Record<string, [number, number, number, number]>;
  }> {
    const frame = this.frameAtSeconds(this.timeS);
    if (this.inspectedFrame !== frame) await this.renderFrame(frame);
    const inspection = this.frameInspection.motion.find((value) => value.clipId === clipId);
    if (!inspection) {
      const active = this.frameInspection.motion.map((value) => value.clipId).join(", ") || "none";
      throw new Error(
        `motion clip '${clipId}' is inactive at frame ${frame}; active Motion clips: ${active}`,
      );
    }
    return {
      object: this.pickScene3d(canvasX, canvasY, clipId),
      compositionFrame: inspection.compositionFrame,
      sourceFrame: inspection.sourceFrame,
      boxes: inspection.boxes,
    };
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.pause();
    if (this.renderInFlight) await this.renderInFlight.catch(() => undefined);
    this.closed = true;
    this.resourceObjects.dispose();
    this.planner?.close();
    this.planner = null;
    this.engine?.free();
    this.renderId = "";
    this.renderReceipt = null;
    this.compiledAudioProgram = null;
    this.stats.renderId = "";
    this.compositor?.dispose();
    for (const ring of this.videoRings.values()) ring.close();
    for (const runtime of this.lottie.values()) {
      runtime.animation.delete();
      runtime.surface.delete();
    }
    this.target?.surface.delete();
    this.target?.context?.delete();
    this.target = null;
    this.scene3dResourceTasks.clear();
    this.videoRings.clear();
    this.lottie.clear();
    this.assetBytes.clear();
    this.fontBytes.clear();
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

  private async stageRenderPackage(next: RenderPackageReplacement): Promise<StagedRenderPackage> {
    const canonical = this.canonicalizeTimelineDocument(
      JSON.parse(next.timelineJson) as TimelineDocument,
    );
    if (next.timelineJson !== canonical.timelineJson) {
      throw new Error("timelineJson must contain the exact canonical Timeline bytes");
    }
    const fixedPackageManifestJson = next.fixedPackageManifestJson;
    const resourceManifestJson = next.resourceManifestJson;
    const verifiedBindingBundleJson = next.verifiedBindingBundleJson;
    const resourceManifest = JSON.parse(resourceManifestJson) as ResourceManifest;
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
      const receipt = parseProductRenderReceipt(engine.open_fixed_package(
        bootstrap.fixedPackageManifestJson,
        bootstrap.timelineJson,
        bootstrap.resourceManifestJson,
        bootstrap.verifiedBindingBundleJson,
      ));
      frozenResources = indexCompiledExecutionResources(engine, receipt.renderId);
      bootstrap.expectedRenderId = receipt.renderId;
      const audioProgram = parseCompiledAudioProgram(JSON.parse(
        engine.audio_program_json(receipt.renderId),
      ));
      if (
        audioProgram.sampleRate !== receipt.sampleRate
        || audioProgram.sampleCount !== receipt.sampleCount
      ) {
        throw new Error("compiled audio program does not match the opened render receipt");
      }
      planner = new ProductFramePlannerPool({
        workerUrl: this.runtimeAssets.workers.productFrame,
        size: 1,
        allocateGeneration: () => this.allocateGeneration(),
      });
      await planner.init(bootstrap);
      compositor = new CanvasKitExecutor(this.CanvasKit, engine);
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

    if (previousRuntime.engine || previousRuntime.planner || previousRuntime.executor) {
      this.resourceObjects.invalidateGeneration();
    }
    for (const ring of this.videoRings.values()) ring.close();
    for (const runtime of this.lottie.values()) {
      runtime.animation.delete();
      runtime.surface.delete();
    }
    this.scene3dResourceTasks.clear();
    this.videoRings.clear();
    this.lottie.clear();
    this.assetBytes.clear();
    this.audioBuffers.clear();
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

  private async fulfillRequests(packet: Uint8Array, gpu: boolean): Promise<FulfilledFrame> {
    const decoded = await decodePackedAbi(packet, RESOURCE_REQUESTS_ABI);
    if (!Array.isArray(decoded)) throw new Error("ResourceRequestSet must decode to an array");
    this.resourceObjects.beginFrame();
    try {
      const objects = new Map<number, CanvasKitExternalObject>();
      const dispose: Array<() => void> = [];
      const requests = decoded.map((value) => {
        const request = record(value, "resource request") as PackedValue & Wire;
        return {
          request,
          handle: exactInteger(request.handle, "resource handle"),
          identity: resourceRequestCacheIdentity(request),
        };
      });
      const groups = new Map<string, {
        request: PackedValue & Wire;
        cached: CachedBrowserResource | undefined;
        produced: ProducedBrowserResource | null;
      }>();
      for (const { identity, request } of requests) {
        if (!groups.has(identity)) {
          groups.set(identity, { request, cached: undefined, produced: null });
        }
      }
      for (const [identity, group] of groups) {
        group.cached = this.resourceObjects.lookup(identity);
      }
      const produced = await Promise.all([...groups.values()].map(async (group) => {
        if (group.cached !== undefined) return null;
        try {
          return { value: await this.fulfillRequestUncached(group.request, gpu) } as const;
        } catch (error) {
          return { error } as const;
        }
      }));
      const failure = produced.find(
        (result): result is { readonly error: unknown } => result !== null && "error" in result,
      );
      if (failure) {
        disposeAll(produced.flatMap((result) => (
          result && "value" in result && result.value ? [result.value.dispose] : []
        )));
        throw failure.error;
      }
      for (const [index, group] of [...groups.values()].entries()) {
        const generated = produced[index];
        group.produced = generated && "value" in generated && generated.value
          ? generated.value
          : null;
      }

      const resolved = new Map<string, CachedBrowserResource>();
      const pendingDisposals = new Set(
        [...groups.values()].flatMap((group) => group.produced ? [group.produced.dispose] : []),
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
              lottieFrames: owned.lottieFrames,
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
        let lottieFrames = 0;
        const seen = new Set<string>();
        for (const { handle, identity } of requests) {
          const value = resolved.get(identity);
          if (!value) throw new Error("resource fulfillment result is missing");
          if (seen.has(identity)) this.resourceObjects.recordFrameReuse(identity);
          else seen.add(identity);
          if (objects.has(handle)) throw new Error(`resource handle ${handle} is duplicated`);
          objects.set(handle, value.object);
          videoFrames += value.videoFrames;
          videoTextureFrames += value.videoTextureFrames;
          lottieFrames += value.lottieFrames;
        }
        const resourceCache = this.resourceObjects.finishFrame(requests.length);
        return {
          objects,
          dispose,
          videoFrames,
          videoTextureFrames,
          lottieFrames,
          resourceCache,
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
  ): Promise<ProducedBrowserResource> {
    const key = record(request.key, "resource key") as PackedValue & Wire;
    const expected = record(request.expected, "resource expected descriptor");
    const interpretation = record(key.interpretation, "resource interpretation");
    const contentWire = contentDigestWire(key.content, "resource key content");
    const content = contentWire.slice("sha256:".length);
    const empty = { videoFrames: 0, videoTextureFrames: 0, lottieFrames: 0 };
    if (expected.kind === "visualFrame") {
      assertCanvaskitImportLayout(expected);
      const asset = this.assetByDigest.get(content);
      if (!asset) throw new Error(`visual resource ${content} has no admitted browser asset`);
      const extent = record(expected.extent, "visual resource extent");
      const bytes = pixelBytes(extent.width, extent.height, 4);
      const sample = record(request.sample, "resource sample");
      if (asset.type === "video") {
        const time = rationalSeconds(record(sample.time, "video source time"));
        const result = await this.videoImage(asset, time, gpu);
        return {
          object: { key, kind: "visual", image: result.image },
          bytes,
          dispose: result.dispose,
          videoFrames: 1,
          videoTextureFrames: Number(result.texture),
          lottieFrames: 0,
        };
      }
      if (asset.type === "lottie") {
        const time = rationalSeconds(record(sample.time, "Lottie source time"));
        const image = await this.lottieImage(asset, time);
        return {
          object: { key, kind: "visual", image },
          bytes,
          dispose: () => image.delete(),
          videoFrames: 0,
          videoTextureFrames: 0,
          lottieFrames: 1,
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
    if (expected.kind === "fontBytes") {
      if (interpretation.kind !== "fontFace") throw new Error("font request has wrong interpretation");
      const bytes = this.fontBytes.get(content);
      if (!bytes) throw new Error(`font bytes ${content} are not registered`);
      return { object: { key, kind: "font", bytes }, bytes: bytes.byteLength, dispose: noop, ...empty };
    }
    if (expected.kind === "runtimeShader") {
      const abi = contentDigestHex(interpretation.abiDigest, "runtime shader ABI digest");
      const bytes = this.shaderBytes.get(`${content}:${abi}`);
      if (!bytes) throw new Error(`runtime shader ${content}:${abi} is not registered`);
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
        payload.kind !== "scene3dFrame"
        || payloadKeys.length !== 2
        || payloadKeys[0] !== "canonicalRequest"
        || payloadKeys[1] !== "kind"
      ) {
        throw new Error("Scene3D request has an invalid payload shape");
      }
      const canonicalRequest = packedByteArray(
        payload.canonicalRequest,
        "Scene3D canonical request",
      );
      await this.ensureScene3dResources(canonicalRequest);
      const topology = contentDigestWire(
        interpretation.topologyDigest,
        "Scene3D topology digest",
      );
      const rgba = this.engine.render_scene3d_request(contentWire, topology, canonicalRequest);
      const width = this.engine.scene3d_frame_width(contentWire);
      const height = this.engine.scene3d_frame_height(contentWire);
      const bytes = pixelBytes(width, height, 4);
      if (rgba.byteLength !== bytes) {
        throw new Error(`Scene3D ${content} returned an invalid RGBA plane`);
      }
      const image = this.CanvasKit.MakeImage({
        width,
        height,
        colorType: this.CanvasKit.ColorType.RGBA_8888,
        alphaType: this.CanvasKit.AlphaType.Premul,
        colorSpace: this.CanvasKit.ColorSpace.SRGB,
      }, rgba, width * 4);
      if (!image) throw new Error(`CanvasKit cannot wrap Scene3D frame ${content}`);
      return {
        object: { key, kind: "scene3d", image },
        bytes,
        dispose: () => image.delete(),
        ...empty,
      };
    }
    throw new Error(`unsupported browser resource descriptor '${String(expected.kind)}'`);
  }

  private async ensureScene3dResources(canonicalRequest: Uint8Array): Promise<void> {
    const needs = JSON.parse(this.engine.scene3d_resource_needs_json(canonicalRequest)) as {
      models: string[];
      textures: string[];
    };
    await Promise.all([
      ...needs.models.map((digest) => this.ensureScene3dResource("model", digest)),
      ...needs.textures.map((digest) => this.ensureScene3dResource("texture", digest)),
    ]);
  }

  private async ensureScene3dResource(kind: "model" | "texture", declaredDigest: string): Promise<void> {
    const digestWire = contentDigestWire(declaredDigest, `Scene3D ${kind} digest`);
    const digest = digestWire.slice("sha256:".length);
    const key = `${kind}:${digest}`;
    const existing = this.scene3dResourceTasks.get(key);
    if (existing) return existing;
    const task = (async () => {
      if (kind === "model") {
        throw new Error(
          `Scene3D model ${digest} cannot be fulfilled by the Timeline common profile`,
        );
      }
      const asset = this.assetByDigest.get(digest);
      if (!asset) throw new Error(`Scene3D ${kind} ${digest} has no admitted browser asset`);
      const bytes = await this.fetchAssetBytes(asset, digest);
      const image = await this.staticImage(asset);
      try {
        const width = image.width();
        const height = image.height();
        const pixels = image.readPixels(0, 0, {
          width,
          height,
          colorType: this.CanvasKit.ColorType.RGBA_8888,
          alphaType: this.CanvasKit.AlphaType.Premul,
          colorSpace: this.CanvasKit.ColorSpace.SRGB,
        });
        if (!(pixels instanceof Uint8Array) || pixels.byteLength !== width * height * 4) {
          throw new Error(`CanvasKit cannot read premultiplied RGBA8 for Scene3D texture '${asset.id}'`);
        }
        this.engine.register_scene3d_texture(digestWire, bytes, width, height, pixels);
      } finally {
        image.delete();
      }
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
    const image = this.CanvasKit.MakeImageFromEncoded(await this.fetchAssetBytes(asset));
    if (!image) throw new Error(`CanvasKit cannot decode image asset '${asset.id}'`);
    return image;
  }

  private async videoImage(
    asset: AdmittedBrowserAsset,
    timeS: number,
    gpu: boolean,
  ): Promise<{ image: Image; texture: boolean; dispose: () => void }> {
    const started = performance.now();
    const ring = await this.videoRing(asset);
    const frame = await ring.frameAt(timeS);
    this.stats.perfVideoDecodeMs += performance.now() - started;
    if (gpu) {
      const image = makeLazyVideoFrameImage(this.CanvasKit, frame, {
        width: frame.displayWidth,
        height: frame.displayHeight,
        colorType: this.CanvasKit.ColorType.RGBA_8888,
        alphaType: this.CanvasKit.AlphaType.Opaque,
      });
      if (image) return { image, texture: true, dispose: () => { image.delete(); frame.close(); } };
    }
    try {
      const rgba = await videoFrameToRgba(frame);
      const image = this.CanvasKit.MakeImage({
        width: rgba.width,
        height: rgba.height,
        colorType: this.CanvasKit.ColorType.RGBA_8888,
        alphaType: this.CanvasKit.AlphaType.Unpremul,
        colorSpace: this.CanvasKit.ColorSpace.SRGB,
      }, rgba.data, rgba.width * 4);
      if (!image) throw new Error(`CanvasKit cannot wrap video frame for '${asset.id}'`);
      return { image, texture: false, dispose: () => image.delete() };
    } finally {
      frame.close();
    }
  }

  private async videoRing(asset: AdmittedBrowserAsset): Promise<DecoderRing> {
    let ring = this.videoRings.get(asset.id);
    if (!ring) {
      const bytes = await this.fetchAssetBytes(asset);
      ring = createDecoderRing({ buffer: Uint8Array.from(bytes).buffer });
      this.videoRings.set(asset.id, ring);
    }
    return ring;
  }

  private async lottieImage(asset: AdmittedBrowserAsset, timeS: number): Promise<Image> {
    let runtime = this.lottie.get(asset.id);
    if (!runtime) {
      if (typeof this.CanvasKit.MakeAnimation !== "function") {
        throw new Error("CanvasKit full runtime with Skottie is required for Lottie");
      }
      const text = new TextDecoder().decode(await this.fetchAssetBytes(asset));
      const json = JSON.parse(text) as Record<string, unknown>;
      const animation = this.CanvasKit.MakeAnimation(text);
      if (!animation) throw new Error(`CanvasKit cannot parse Lottie asset '${asset.id}'`);
      const size = animation.size();
      const width = Math.max(1, Math.ceil(size[0] || finiteOr(asset.descriptor.width, finiteOr(json.w, 1))));
      const height = Math.max(1, Math.ceil(size[1] || finiteOr(asset.descriptor.height, finiteOr(json.h, 1))));
      const surface = this.CanvasKit.MakeSurface(width, height);
      if (!surface) throw new Error(`CanvasKit cannot allocate Lottie surface ${width}x${height}`);
      runtime = { animation, surface, width, height, fps: finiteOr(json.fr, 60) };
      this.lottie.set(asset.id, runtime);
    }
    runtime.surface.getCanvas().clear(this.CanvasKit.TRANSPARENT);
    runtime.animation.seekFrame(timeS * runtime.fps);
    runtime.animation.render(
      runtime.surface.getCanvas(),
      this.CanvasKit.XYWHRect(0, 0, runtime.width, runtime.height),
    );
    runtime.surface.flush();
    return runtime.surface.makeImageSnapshot();
  }

  private acquireTarget(width: number, height: number, forceCpu: boolean): TargetSurface {
    const gpu = this.gpu && !forceCpu && typeof document !== "undefined";
    const key = `${gpu ? "gpu" : "cpu"}:${width}x${height}`;
    if (this.target?.key === key) return this.target;
    if (this.target) this.resourceObjects.invalidateGeneration();
    this.target?.surface.delete();
    this.target?.context?.delete();
    if (gpu) {
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const handle = this.CanvasKit.GetWebGLContext(canvas, { preserveDrawingBuffer: 1 });
      const context = handle ? this.CanvasKit.MakeWebGLContext(handle) : null;
      const surface = context
        ? this.CanvasKit.MakeOnScreenGLSurface(context, width, height, this.CanvasKit.ColorSpace.SRGB)
        : null;
      if (surface && context) {
        this.target = { key, surface, gpu: true, context, canvas };
        return this.target;
      }
      context?.delete();
    }
    const surface = this.CanvasKit.MakeSurface(width, height);
    if (!surface) throw new Error(`CanvasKit cannot allocate product target ${width}x${height}`);
    this.target = { key: `cpu:${width}x${height}`, surface, gpu: false, context: null, canvas: null };
    return this.target;
  }

  private presentTarget(target: TargetSurface, width: number, height: number): void {
    if (!this.canvas) return;
    if (this.canvas.width !== width) this.canvas.width = width;
    if (this.canvas.height !== height) this.canvas.height = height;
    const context = this.canvas.getContext("2d");
    if (!context) throw new Error("visible canvas 2D context is unavailable");
    if (target.gpu && target.canvas) {
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
    if (!(pixels instanceof Uint8Array)) throw new Error("CanvasKit target readback failed");
    context.putImageData(new ImageData(Uint8ClampedArray.from(pixels), width, height), 0, 0);
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
    const receipt = this.requireRenderReceipt();
    const fps = canonicalRationalNumber(receipt.frameRate, "frameRate");
    const lastFrame = receipt.frameCount - 1;
    const clockFrame = this.frameAtSeconds(this.timeS);
    let frame = Math.max(
      clockFrame,
      this.playbackFrame === null ? clockFrame : this.playbackFrame + 1,
    );
    while (this.playing && !this.closed && generation === this.playGeneration && frame <= lastFrame) {
      this.prefetchPlanningWindow(frame);
      const dueTime = frame / fps;
      while (this.playing && !this.closed && generation === this.playGeneration) {
        const remainingMs = (dueTime - this.clock.now()) * 1000;
        if (remainingMs <= 0.5) break;
        await waitMilliseconds(Math.min(8, remainingMs));
      }
      if (!this.playing || this.closed || generation !== this.playGeneration) return;
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
    if (!this.playing || this.closed || generation !== this.playGeneration) return;
    this.pause();
    this.timeS = this.lastFrameTimeS();
    this.onTimeUpdate?.(this.timeS);
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

  private audioBufferForDigest(
    context: BaseAudioContext,
    declaredDigest: ContentDigestWire,
    decodedPcmDigest: string,
    sourceChannels: 1 | 2,
  ): Promise<AudioBuffer> {
    const digest = contentDigestHex(declaredDigest, "audio resource digest");
    const expectedPcm = contentDigestHex(decodedPcmDigest, "decoded PCM digest");
    const key = `${digest}@${context.sampleRate}/${sourceChannels}/${expectedPcm}`;
    let promise = this.audioBuffers.get(key);
    if (!promise) {
      const asset = this.assetByDigest.get(digest);
      if (!asset) throw new Error(`compiled audio resource sha256:${digest} has no browser source`);
      promise = this.fetchAssetBytes(asset, digest)
        .then((bytes) => context.decodeAudioData(bytes.slice().buffer))
        .then(async (buffer) => {
          if (buffer.sampleRate !== context.sampleRate || buffer.numberOfChannels !== sourceChannels) {
            throw new Error(
              `decoded audio sha256:${digest} is ${buffer.sampleRate} Hz/${buffer.numberOfChannels} channels; expected ${context.sampleRate} Hz/${sourceChannels}`,
            );
          }
          const actualPcm = await commonAudioPcmDigestHex(buffer);
          if (actualPcm !== expectedPcm) {
            throw new Error(
              `decoded audio sha256:${digest} PCM digest mismatch: expected ${expectedPcm}, got ${actualPcm}`,
            );
          }
          return buffer;
        });
      this.audioBuffers.set(key, promise);
    }
    return promise;
  }

  private async renderCompiledAudioBuffer(
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
      !Number.isSafeInteger(startSample)
      || !Number.isSafeInteger(endSample)
      || startSample < 0
      || endSample <= startSample
      || endSample > program.sampleCount
    ) {
      throw new RangeError(`invalid compiled audio range [${startSample}, ${endSample})`);
    }
    const output = context.createBuffer(2, endSample - startSample, program.sampleRate);
    const leftOutput = output.getChannelData(0);
    const rightOutput = output.getChannelData(1);
    const blockSize = 8_192;
    for (let blockStart = startSample; blockStart < endSample; blockStart += blockSize) {
      const blockEnd = Math.min(endSample, blockStart + blockSize);
      const block = parseAudioBlock(JSON.parse(this.engine.audio_block_json(
        this.renderId,
        BigInt(blockStart),
        BigInt(blockEnd),
      )));
      if (
        block.renderId !== this.renderId
        || block.sampleRate !== program.sampleRate
        || block.startSample !== blockStart
        || block.endSample !== blockEnd
        || block.samples.length !== blockEnd - blockStart
      ) {
        throw new Error("compiled audio block does not match the active RenderId/range");
      }
      const requirements = new Map<string, {
        declaredDigest: ContentDigestWire;
        decodedPcmDigest: string;
        sourceChannels: 1 | 2;
      }>();
      for (const sample of block.samples) {
        for (const track of sample.tracks) {
          for (const endpoint of track.endpoints) {
            if (!endpoint.digest) throw new Error(`audio source ${endpoint.sourceIndex} has no admitted digest`);
            const declaredDigest = contentDigestWire(
              endpoint.digest,
              "audio endpoint resource digest",
            );
            const digest = declaredDigest.slice("sha256:".length);
            const prior = requirements.get(digest);
            if (prior && (prior.decodedPcmDigest !== endpoint.decodedPcmDigest
              || prior.sourceChannels !== endpoint.sourceChannels)) {
              throw new Error(`audio source sha256:${digest} has conflicting decoded PCM proofs`);
            }
            requirements.set(digest, {
              declaredDigest,
              decodedPcmDigest: endpoint.decodedPcmDigest,
              sourceChannels: endpoint.sourceChannels,
            });
          }
        }
      }
      const decoded = new Map<string, AudioBuffer>();
      await Promise.all([...requirements].map(async ([digest, proof]) => {
        decoded.set(digest, await this.audioBufferForDigest(
          context,
          proof.declaredDigest,
          proof.decodedPcmDigest,
          proof.sourceChannels,
        ));
      }));
      const mixed = mixCommonAudioBlockPcm(block, decoded);
      const outputOffset = blockStart - startSample;
      leftOutput.set(mixed.left, outputOffset);
      rightOutput.set(mixed.right, outputOffset);
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
      this.playing
      && !this.closed
      && generation === this.playGeneration
      && this.audioGraph === graph
      && graph.nextSample < program.sampleCount
    ) {
      if (graph.scheduledUntilContextTime - context.currentTime > 0.5) {
        await waitMilliseconds(50);
        continue;
      }
      const startSample = graph.nextSample;
      const endSample = Math.min(program.sampleCount, startSample + program.sampleRate);
      const buffer = await this.renderCompiledAudioBuffer(context, startSample, endSample);
      if (
        !this.playing
        || this.closed
        || generation !== this.playGeneration
        || this.audioGraph !== graph
      ) return;
      const source = context.createBufferSource();
      source.buffer = buffer;
      source.connect(graph.master);
      const when = Math.max(context.currentTime, graph.scheduledUntilContextTime);
      source.start(when);
      graph.sources.push(source);
      graph.nextSample = endSample;
      graph.scheduledUntilContextTime = when + buffer.duration;
      this.stats.audioScheduledSources += 1;
    }
  }

  private teardownAudioGraph(): void {
    if (!this.audioGraph) return;
    for (const source of this.audioGraph.sources) {
      try { source.stop(); } catch { /* already ended */ }
      source.disconnect();
    }
    this.audioGraph.master.disconnect();
    this.audioGraph = null;
  }

  private applyMasterVolume(): void {
    if (this.audioGraph) this.audioGraph.master.gain.value = this.muted ? 0 : this.volume;
  }

  private async fetchAssetBytes(asset: AdmittedBrowserAsset, expectedDigest?: string): Promise<Uint8Array> {
    const url = this.assetUrl(asset);
    let promise = this.assetBytes.get(url);
    if (!promise) {
      promise = fetchBytes(url);
      this.assetBytes.set(url, promise);
    }
    const bytes = await promise;
    const expected = expectedDigest ?? contentDigestHex(asset.contentDigest, `asset '${asset.id}' digest`);
    if (expected) {
      const actual = await sha256Hex(bytes);
      if (actual !== expected) throw new Error(`asset '${asset.id}' digest mismatch: expected ${expected}, got ${actual}`);
    }
    return bytes;
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
    if (type && asset.type !== type) throw new Error(`asset '${id}' is not ${type}`);
    return asset;
  }

  private requireRenderReceipt(): ProductRenderReceipt {
    if (!this.renderReceipt) throw new Error("browser player has no open fixed render");
    return this.renderReceipt;
  }

  private requireCompiledAudioProgram(): CompiledAudioProgramWire {
    if (!this.compiledAudioProgram) throw new Error("browser player has no compiled audio program");
    return this.compiledAudioProgram;
  }

  private frameAtSeconds(seconds: number): number {
    if (!this.renderId) throw new Error("browser player has no open RenderId");
    return exactBigIntNumber(
      this.engine.frame_at_seconds(this.renderId, seconds),
      "compiled frame clock",
    );
  }

  private sampleAtSeconds(seconds: number, allowEnd = false): number {
    const program = this.requireCompiledAudioProgram();
    if (allowEnd && seconds >= this.durationS()) return program.sampleCount;
    if (!this.renderId) throw new Error("browser player has no open RenderId");
    return exactBigIntNumber(
      this.engine.sample_at_seconds(this.renderId, seconds),
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
    const surfaceBytes = BigInt(width) * BigInt(height) * 8n;
    const maxSurfaceBytes = maxBigInt(surfaceBytes, 64n * 1024n * 1024n);
    return {
      frame,
      width,
      height,
      transparent,
      maxSurfaceBytes,
      maxFrameBytes: maxBigInt(maxSurfaceBytes * 16n, 512n * 1024n * 1024n),
    };
  }

  private prefetchPlanningWindow(currentFrame: number): ProductFramePlanningJob[] {
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
      jobs.push(planner.prefetch(this.framePlanningInput(candidate, width, height, false), offset));
    }
    this.syncPlannerStats();
    return jobs;
  }

  private requirePlanner(): ProductFramePlannerPool {
    if (!this.planner) throw new Error("Product frame planner is not initialized");
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

  private pickScene3d(
    canvasX: number,
    canvasY: number,
    clipId?: string,
  ): { semanticAddress: string } | null {
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
      if (width <= 0 || height <= 0
        || local[0] < x || local[1] < y
        || local[0] >= x + width || local[1] >= y + height) continue;
      const frameWidth = this.engine.scene3d_frame_width(placement.contentHash);
      const frameHeight = this.engine.scene3d_frame_height(placement.contentHash);
      const pixelX = Math.min(frameWidth - 1, Math.floor((local[0] - x) / width * frameWidth));
      const pixelY = Math.min(frameHeight - 1, Math.floor((local[1] - y) / height * frameHeight));
      const pick = JSON.parse(
        this.engine.scene3d_pick_json(placement.contentHash, pixelX, pixelY),
      ) as { semanticAddress: string } | null;
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
    if (byId.has(asset.id)) throw new Error(`asset '${asset.id}' is duplicated`);
    const entry = manifest.entries[asset.id];
    if (!entry) {
      throw new Error(`browser locator '${asset.id}' is absent from the pinned ResourceManifest`);
    }
    const digest = contentDigestHex(entry.digest, `resource '${asset.id}' digest`);
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

function indexCompiledExecutionResources(
  engine: ProductEngineWire,
  renderId: string,
): FrozenBundleResources {
  const fonts = new Map<string, Uint8Array>();
  const shaders = new Map<string, Uint8Array>();
  const resources = JSON.parse(engine.compiled_execution_resources_json(renderId));
  if (!Array.isArray(resources)) {
    throw new Error("compiled execution resource index must be an array");
  }
  for (const rawResource of resources) {
    const resource = record(rawResource, "compiled execution resource");
    if (typeof resource.resourceId !== "string" || resource.resourceId.length === 0) {
      throw new Error("compiled execution resource id must be non-empty");
    }
    if (
      typeof resource.contentDigest !== "string"
      || !/^sha256:[0-9a-f]{64}$/.test(resource.contentDigest)
    ) {
      throw new Error(`compiled execution resource '${resource.resourceId}' has an invalid digest`);
    }
    if (
      resource.kind !== "font-bytes"
      && resource.kind !== "runtime-shader"
    ) {
      throw new Error(
        `compiled execution resource '${resource.resourceId}' has unknown kind '${String(resource.kind)}'`,
      );
    }
    if (resource.kind === "runtime-shader") {
      if (
        typeof resource.abiDigest !== "string"
        || !/^sha256:[0-9a-f]{64}$/.test(resource.abiDigest)
      ) {
        throw new Error(`compiled runtime shader '${resource.resourceId}' has an invalid ABI digest`);
      }
    } else if (resource.abiDigest != null) {
      throw new Error(`compiled resource '${resource.resourceId}' unexpectedly declares an ABI digest`);
    }
    const bytes = engine.compiled_resource_bytes(
      renderId,
      resource.kind,
      resource.contentDigest,
      resource.abiDigest,
    );
    const content = contentDigestHex(
      resource.contentDigest,
      `compiled resource '${resource.resourceId}' content digest`,
    );
    if (resource.kind === "font-bytes") {
      fonts.set(content, bytes);
    } else if (resource.kind === "runtime-shader") {
      const abi = contentDigestHex(
        resource.abiDigest,
        `compiled resource '${resource.resourceId}' ABI digest`,
      );
      shaders.set(`${content}:${abi}`, bytes);
    }
  }
  return { fonts, shaders };
}

function replaceMap<K, V>(target: Map<K, V>, source: ReadonlyMap<K, V>): void {
  target.clear();
  for (const [key, value] of source) target.set(key, value);
}

function emptyPlayerStats(): PlayerStats {
  return {
    rAFFrames: 0, renders: 0, scrubs: 0, fontsRegistered: 0,
    videoFrames: 0, gpuVideoFrames: 0, lottieFrames: 0,
    clockMode: "performance", surfaceMode: "none",
    gpuFrames: 0, cpuFrames: 0, audioMode: "none", audioScheduledSources: 0,
    audioDecodeErrors: 0, audioPeaksDecodes: 0,
    preparedFrames: 0, perfFrameMs: 0, perfEvaluatePrepareMs: 0,
    perfRequestInspectMs: 0, perfResourceFulfillMs: 0, perfLowerMs: 0,
    perfBindPacketsMs: 0, perfExecuteMs: 0,
    perfExecutorPacketAdmissionMs: 0, perfExecutorProgramAdmissionMs: 0,
    perfExecutorScheduleAdmissionMs: 0, perfExecutorRenderMs: 0,
    programLocalPasses: 0, directRasterPrograms: 0, programGroups: 0,
    programTransformGroups: 0, programClipGroups: 0, programOpacityGroups: 0,
    programFilterGroups: 0, programMaskGroups: 0, programShaderGroups: 0,
    programBackdropGroups: 0, programBlendGroups: 0,
    perfPresentMs: 0, perfCaptureEncodeMs: 0,
    perfVideoDecodeMs: 0, perfVideoCopyMs: 0, perfVideoWrapMs: 0,
    videoThumbExtracts: 0, planBytes: 0, bindingBytes: 0, scheduleBytes: 0, requestBytes: 0,
    executionPasses: 0, physicalSurfaces: 0, maximumLiveImages: 0,
    surfaceAllocations: 0, surfaceReuses: 0, surfaceAllocatedBytes: 0,
    maximumSurfaceResidentBytes: 0, maximumSurfacePeakBytes: 0, surfacePoolEvictions: 0,
    templateCacheHits: 0, templateCacheMisses: 0,
    programCacheHits: 0, programCacheMisses: 0,
    fontCacheHits: 0, fontCacheMisses: 0,
    shaderCacheHits: 0, shaderCacheMisses: 0,
    resourceCacheGenerationInvalidations: 0,
    resourceCacheHits: 0, resourceCacheMisses: 0,
    resourceCacheInsertions: 0, resourceCacheEvictions: 0, resourceCacheBypasses: 0,
    maximumResourceCacheEntries: 0, maximumResourceCacheBytes: 0,
    renderId: "",
    framePlannerWorkers: 0, framePlannerReadyHits: 0, framePlannerWaitHits: 0,
    framePlannerQueued: 0, framePlannerCompleted: 0, framePlannerCancelled: 0,
    framePlannerErrors: 0, framePlannerQueueDepth: 0, framePlannerReadyFrames: 0,
    framePlannerReadyBytes: 0, perfFramePlannerWorkerMs: 0,
    perfFramePlannerTurnGapMs: 0,
    perfFramePlannerReleaseMs: 0,
    motionExecuteClips: {}, motionPerformance: {}, degradations: [],
  };
}

async function loadWasmModule(moduleUrl: string, wasmUrl: string): Promise<ValleWasmModule> {
  const module = await import(moduleUrl) as unknown as ValleWasmModule;
  await module.default(wasmUrl);
  return module;
}

async function loadCanvasKit(assets: PlayerRuntimeAssets["canvasKit"]): Promise<CanvasKit> {
  canvasKitPromise ??= (async () => {
    const init = staticCanvasKitInit ?? await injectCanvasKitScript(assets.full.glue);
    return init({
      locateFile: (file) => file.endsWith(".wasm")
        ? assets.full.wasm
        : new URL(file, assets.full.glue).href,
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
    script.onload = () => typeof globalThis.CanvasKitInit === "function"
      ? resolve(globalThis.CanvasKitInit)
      : reject(new Error(`CanvasKitInit missing after loading ${src}`));
    script.onerror = () => reject(new Error(`failed to load CanvasKit script ${src}`));
    document.head.appendChild(script);
  });
}

async function fetchBytes(url: string): Promise<Uint8Array> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`fetch ${url}: HTTP ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const buffer = bytes.slice().buffer as ArrayBuffer;
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", buffer));
  return [...digest].map((value) => value.toString(16).padStart(2, "0")).join("");
}

export async function commonAudioPcmDigestHex(buffer: AudioBuffer): Promise<string> {
  const domain = new TextEncoder().encode("valle.audio/common@1/decoded-interleaved-f32le@1\0");
  const headerBytes = 4 + 2 + 8;
  const sampleBytes = buffer.length * buffer.numberOfChannels * 4;
  if (!Number.isSafeInteger(sampleBytes)) {
    throw new Error("decoded audio PCM exceeds the Web hashing range");
  }
  const bytes = new Uint8Array(domain.length + headerBytes + sampleBytes);
  bytes.set(domain);
  const view = new DataView(bytes.buffer);
  let offset = domain.length;
  view.setUint32(offset, buffer.sampleRate, true);
  offset += 4;
  view.setUint16(offset, buffer.numberOfChannels, true);
  offset += 2;
  view.setBigUint64(offset, BigInt(buffer.length), true);
  offset += 8;
  const channels = Array.from(
    { length: buffer.numberOfChannels },
    (_, channel) => buffer.getChannelData(channel),
  );
  for (let frame = 0; frame < buffer.length; frame += 1) {
    for (const channel of channels) {
      const sample = channel[frame]!;
      if (!Number.isFinite(sample)) throw new Error("decoded audio PCM contains a non-finite sample");
      view.setFloat32(offset, sample, true);
      offset += 4;
    }
  }
  return sha256Hex(bytes);
}

/**
 * Deterministic common-profile PCM mix used by production playback and bit-exact Web goldens.
 * Engine has already frozen effects, gain, pan, and crossfade into each endpoint's left/right
 * gain. This stage only accumulates admitted f32 source PCM in packet order using f64 JavaScript
 * numbers, clamps, and rounds once when assigning the terminal Float32Array.
 */
export function mixCommonAudioBlockPcm(
  block: AudioBlockWire,
  decoded: ReadonlyMap<string, Pick<AudioBuffer, "length" | "numberOfChannels" | "getChannelData">>,
): { left: Float32Array; right: Float32Array } {
  const frameCount = block.endSample - block.startSample;
  if (!Number.isSafeInteger(frameCount) || frameCount < 0 || block.samples.length !== frameCount) {
    throw new Error("compiled audio block has an invalid sample range");
  }
  const leftOutput = new Float32Array(frameCount);
  const rightOutput = new Float32Array(frameCount);
  for (let index = 0; index < block.samples.length; index += 1) {
    const sample = block.samples[index]!;
    const expectedSample = block.startSample + index;
    if (sample.renderId !== block.renderId || sample.sample !== expectedSample) {
      throw new Error(`compiled audio sample identity drift at ${expectedSample}`);
    }
    let left = 0;
    let right = 0;
    for (const track of sample.tracks) {
      for (const endpoint of track.endpoints) {
        const digest = endpoint.digest == null
          ? null
          : contentDigestHex(endpoint.digest, "compiled audio endpoint digest");
        const source = digest ? decoded.get(digest) : undefined;
        if (!source) throw new Error(`audio source ${endpoint.sourceIndex} is unavailable`);
        if (source.numberOfChannels !== endpoint.sourceChannels) {
          throw new Error(
            `audio source ${endpoint.sourceIndex} channel proof drifted after fulfillment`,
          );
        }
        if (!Number.isSafeInteger(endpoint.sourceSampleIndex)
          || endpoint.sourceSampleIndex < 0
          || endpoint.sourceSampleIndex >= source.length) {
          throw new Error(
            `compiled audio source sample ${endpoint.sourceSampleIndex} is outside decoded source ${source.length}`,
          );
        }
        const sourceFrame = endpoint.sourceSampleIndex;
        const sourceLeft = source.getChannelData(0)[sourceFrame] ?? 0;
        const sourceRight = source.numberOfChannels > 1
          ? source.getChannelData(1)[sourceFrame] ?? 0
          : sourceLeft;
        left += sourceLeft * endpoint.leftGain;
        right += sourceRight * endpoint.rightGain;
      }
    }
    leftOutput[index] = clamp(left, -1, 1);
    rightOutput[index] = clamp(right, -1, 1);
  }
  return { left: leftOutput, right: rightOutput };
}

async function videoFrameToRgba(frame: VideoFrame): Promise<{ width: number; height: number; data: Uint8Array }> {
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
    throw new Error(`${label} must be a sha256:<64 lowercase hex> ContentDigest`);
  }
  return value as ContentDigestWire;
}

function contentDigestHex(value: unknown, label: string): string {
  return contentDigestWire(value, label).slice("sha256:".length);
}

function packedByteArray(value: unknown, label: string): Uint8Array {
  if (!Array.isArray(value)) throw new Error(`${label} must be a byte array`);
  return Uint8Array.from(value.map((item, index) => {
    const number = typeof item === "bigint" ? Number(item) : item;
    if (!Number.isInteger(number) || Number(number) < 0 || Number(number) > 255) {
      throw new Error(`${label}[${index}] is not a byte`);
    }
    return Number(number);
  }));
}

function inverseHomography(matrix: readonly number[]): number[] | null {
  if (matrix.length !== 9 || !matrix.every(Number.isFinite)) return null;
  const [a, b, c, d, e, f, g, h, i] = matrix as [
    number, number, number, number, number, number, number, number, number,
  ];
  const determinant = a * (e * i - f * h)
    - b * (d * i - f * g)
    + c * (d * h - e * g);
  if (!Number.isFinite(determinant) || Math.abs(determinant) <= Number.EPSILON) return null;
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

function applyHomography(matrix: readonly number[], point: readonly [number, number]): [number, number] | null {
  const [x, y] = point;
  const w = matrix[6]! * x + matrix[7]! * y + matrix[8]!;
  if (!Number.isFinite(w) || Math.abs(w) <= Number.EPSILON) return null;
  const mapped: [number, number] = [
    (matrix[0]! * x + matrix[1]! * y + matrix[2]!) / w,
    (matrix[3]! * x + matrix[4]! * y + matrix[5]!) / w,
  ];
  return mapped.every(Number.isFinite) ? mapped : null;
}

function rationalSeconds(value: Wire): number {
  const numerator = typeof value.numerator === "bigint" ? Number(value.numerator) : Number(value.numerator);
  const denominator = Number(value.denominator);
  if (!Number.isSafeInteger(numerator) || !Number.isSafeInteger(denominator) || denominator <= 0) {
    throw new Error("RationalTime is invalid");
  }
  return numerator / denominator;
}

function exactInteger(value: unknown, label: string): number {
  const number = typeof value === "bigint" ? Number(value) : value;
  if (!Number.isSafeInteger(number) || Number(number) <= 0) throw new Error(`${label} must be positive`);
  return Number(number);
}

function pixelBytes(width: unknown, height: unknown, bytesPerPixel: number): number {
  const w = exactInteger(width, "resource width");
  const h = exactInteger(height, "resource height");
  const bytes = w * h * bytesPerPixel;
  if (!Number.isSafeInteger(bytes)) throw new Error("resource byte size overflowed");
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
  if (!Number.isInteger(value) || Number(value) <= 0) throw new Error(`${label} must be positive`);
  return Number(value);
}

function finitePositive(value: unknown, label: string): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    throw new Error(`${label} must be finite and positive`);
  }
  return value;
}

function finiteOr(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function clamp(value: number, low: number, high: number): number {
  return Math.max(low, Math.min(high, Number.isFinite(value) ? value : low));
}

function maxBigInt(left: bigint, right: bigint): bigint {
  return left > right ? left : right;
}

function exactBigIntNumber(value: bigint, label: string): number {
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < 0) throw new Error(`${label} exceeds the exact reporting range`);
  return number;
}

function maybeCreateAudioContext(sampleRate: number): AudioContext | null {
  const Constructor = globalThis.AudioContext ?? globalThis.webkitAudioContext;
  try { return Constructor ? new Constructor({ sampleRate }) : null; } catch { return null; }
}

function waitMilliseconds(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, Math.max(0, milliseconds)));
}

function canonicalRationalNumber(value: string, label: string): number {
  const match = /^(-?(?:0|[1-9][0-9]*))\/([1-9][0-9]*)$/.exec(value);
  if (!match) throw new TypeError(`${label} must be a canonical rational`);
  const numerator = Number(match[1]);
  const denominator = Number(match[2]);
  const result = numerator / denominator;
  if (!Number.isFinite(result)) throw new TypeError(`${label} exceeds the Web numeric range`);
  return result;
}

function parseCompiledAudioProgram(value: unknown): CompiledAudioProgramWire {
  const wire = record(value, "compiled audio program");
  if (!Array.isArray(wire.tracks)) {
    throw new TypeError("compiled audio program must contain tracks");
  }
  for (const field of ["sampleRate", "sampleCount"] as const) {
    if (!Number.isSafeInteger(wire[field]) || Number(wire[field]) <= 0) {
      throw new TypeError(`compiled audio program ${field} must be a positive safe integer`);
    }
  }
  return wire as CompiledAudioProgramWire;
}

function parseAudioBlock(value: unknown): AudioBlockWire {
  const wire = record(value, "compiled audio block");
  if (typeof wire.renderId !== "string" || !Array.isArray(wire.samples)) {
    throw new TypeError("compiled audio block is malformed");
  }
  for (const field of ["sampleRate", "startSample", "endSample"] as const) {
    if (!Number.isSafeInteger(wire[field])) {
      throw new TypeError(`compiled audio block ${field} must be a safe integer`);
    }
  }
  for (const [sampleIndex, sampleValue] of wire.samples.entries()) {
    const sample = record(sampleValue, `compiled audio sample ${sampleIndex}`);
    if (!Array.isArray(sample.tracks)) {
      throw new TypeError(`compiled audio sample ${sampleIndex} must contain tracks`);
    }
    for (const [trackIndex, trackValue] of sample.tracks.entries()) {
      const track = record(trackValue, `compiled audio track ${trackIndex}`);
      if (!Array.isArray(track.endpoints)) {
        throw new TypeError(`compiled audio track ${trackIndex} must contain endpoints`);
      }
      for (const [endpointIndex, endpointValue] of track.endpoints.entries()) {
        const endpoint = record(endpointValue, `compiled audio endpoint ${endpointIndex}`);
        if (typeof endpoint.decodedPcmDigest !== "string"
          || !/^sha256:[0-9a-f]{64}$/.test(endpoint.decodedPcmDigest)) {
          throw new TypeError(`compiled audio endpoint ${endpointIndex} has no decoded PCM proof`);
        }
        if (endpoint.sourceChannels !== 1 && endpoint.sourceChannels !== 2) {
          throw new TypeError(`compiled audio endpoint ${endpointIndex} has invalid source channels`);
        }
      }
    }
  }
  return wire as AudioBlockWire;
}

class AudioMasterClock {
  mode = "performance";
  private mediaTime = 0;
  private startedAt = 0;
  constructor(private readonly context: AudioContext | null) {}

  async start(mediaTime: number, { useAudioClock }: { useAudioClock: boolean }): Promise<void> {
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
    const clock = this.mode === "audio" && this.context
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
