import type {
  EditTimelineRequest,
  EditTimelineResponse,
  Timeline,
} from "@valle/engine";
import type {
  MotionContextWire,
  StudioBootWire,
} from "@valle/engine/protocol";
import { MOTION_SOURCE_MAP_VERSION, STUDIO_HOST_PROTOCOL_VERSION } from "@valle/engine/protocol";
import type {
  ResourceManifest,
  TimelineDocument,
} from "@valle/engine/internal";
import type { PlayerRuntimeAssets } from "@valle/player-core";
import { findMotionStructure } from "./project-motion-edit.ts";

export type StudioBoot = StudioBootWire;
export type StudioSession = StudioBoot["session"];
type MotionContextOk = Extract<MotionContextWire, { status: "ok" }>;

export interface StudioEvent {
  type: "timeline" | "project" | "motion" | "runtime";
  data: unknown;
}

export interface TimelineContext {
  timelineRevision: StudioTimelineRevision;
  timelineJson: string;
  timeline: Timeline;
  /** Derived execution data. Never use this as the Studio working copy. */
  render: TimelineRenderContext;
  preview?: { status: "unavailable"; code: string };
  motion?: unknown;
  assets?: unknown[];
  runtimeAssets: PlayerRuntimeAssets;
  assetBaseUrl: string;
  proxyBase?: string | null;
  generation?: number;
}

export interface StudioTimelineRevision {
  revision: number;
  parentRevision: number | null;
  createdAt: string;
  actor: string;
  cause:
    | { type: "genesis" }
    | { type: "timelineEdit" }
    | { type: "restore"; sourceRevision: number };
  intent: string | null;
}

type TimelineRenderProjection = {
  timelineJson: string;
  timeline: TimelineDocument;
};

export type TimelineRenderContext = TimelineRenderProjection & (
  | {
      fixedPackageManifestJson: string;
      resourceManifestJson: string;
      resourceManifest: ResourceManifest;
      verifiedBindingBundleJson: string;
    }
  | {
      fixedPackageManifestJson?: never;
      resourceManifestJson?: never;
      resourceManifest?: never;
      verifiedBindingBundleJson?: never;
    }
);

export interface MotionRequest {
  clipId?: string;
}

export type MotionContext = MotionContextWire;

type TimelineVisualItem = TimelineDocument["document"]["visual"]["tracks"][number]["items"][number];
type TimelineVisualClip = Extract<TimelineVisualItem, { type: "clip" }>;
type TimelineMotionSource = Extract<TimelineVisualClip["source"], { type: "motion" }>;

export type TimelineEdit = EditTimelineRequest;
export type SaveReport = EditTimelineResponse;

export interface StudioHost {
  readonly boot: StudioBoot;
  load(): Promise<StudioBoot>;
  subscribe(onEvent: (event: StudioEvent) => void): () => void;
  loadTimeline?(): Promise<TimelineContext>;
  saveTimeline?(edit: TimelineEdit): Promise<SaveReport>;
  loadMotion?(request: MotionRequest): Promise<MotionContext>;
}

interface EventSourceLike {
  addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void;
  close(): void;
}

type EventSourceFactory = (url: string) => EventSourceLike;
type Fetcher = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

const studioEventTypes: StudioEvent["type"][] = ["timeline", "project", "motion", "runtime"];

abstract class BaseStudioHost implements StudioHost {
  readonly boot: StudioBoot;
  protected readonly fetcher: Fetcher;
  #eventSourceFactory: EventSourceFactory;

  constructor(
    boot: StudioBoot,
    fetcher: Fetcher = globalThis.fetch.bind(globalThis),
    eventSourceFactory: EventSourceFactory = (url) => new EventSource(url),
  ) {
    this.boot = boot;
    this.fetcher = fetcher;
    this.#eventSourceFactory = eventSourceFactory;
  }

  async load(): Promise<StudioBoot> {
    return this.boot;
  }

  subscribe(onEvent: (event: StudioEvent) => void): () => void {
    const source = this.#eventSourceFactory("/events");
    for (const type of studioEventTypes) {
      source.addEventListener(type, (event) => {
        let data: unknown;
        try {
          data = event.data ? JSON.parse(event.data) as unknown : null;
        } catch {
          source.close();
          throw new TypeError(`Studio ${type} event payload must be valid JSON`);
        }
        onEvent({ type, data });
      });
    }
    return () => source.close();
  }

  protected async fetchJson(url: string, init?: RequestInit): Promise<Record<string, unknown>> {
    const response = await this.fetcher(url, init);
    if (!response.ok) throw new Error(`${url} returned ${response.status}`);
    const value: unknown = await response.json();
    if (!isRecord(value)) throw new Error(`${url} did not return a JSON object`);
    return value;
  }
}

export class ProjectHost extends BaseStudioHost {
  declare readonly boot: StudioBoot & { session: Extract<StudioSession, { kind: "project" }> };

  constructor(
    boot: StudioBoot & { session: Extract<StudioSession, { kind: "project" }> },
    fetcher?: Fetcher,
    eventSourceFactory?: EventSourceFactory,
  ) {
    super(boot, fetcher, eventSourceFactory);
    this.boot = boot;
  }

  async loadTimeline(): Promise<TimelineContext> {
    const config = await this.fetchJson(
      `/timeline/get?project=${encodeURIComponent(this.boot.session.projectId)}`,
    );
    return assertTimelineContext({
      ...config,
      runtimeAssets: playerRuntimeAssetsFromStudioBoot(this.boot),
      assetBaseUrl: this.boot.runtime.assetBaseUrl,
      proxyBase: this.boot.runtime.proxyBase ?? null,
    });
  }

  async saveTimeline(edit: TimelineEdit): Promise<SaveReport> {
    const report = await this.fetchJson(
      `/timeline/edit?project=${encodeURIComponent(this.boot.session.projectId)}`,
      {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-valle-token": this.boot.session.token,
        },
        body: JSON.stringify(edit),
      },
    );
    return report as unknown as SaveReport;
  }

}

export function playerRuntimeAssetsFromStudioBoot(boot: StudioBoot): PlayerRuntimeAssets {
  const urls = boot.runtime.assetUrls;
  const required = (name: string): string => {
    const value = urls[name];
    if (typeof value !== "string" || value.length === 0) {
      throw new Error(`Studio boot runtime.assetUrls.${name} is required`);
    }
    return value;
  };
  return {
    engine: {
      glue: required("engineGlue"),
      wasm: required("engineWasm"),
    },
    canvasKit: {
      full: {
        glue: required("canvasKitFullGlue"),
        wasm: required("canvasKitFullWasm"),
      },
    },
    fonts: { defaultSans: required("defaultSansFont") },
    workers: { productFrame: required("productFrameWorker") },
  };
}

export class TimelineFileHost extends BaseStudioHost {
  async loadTimeline(): Promise<TimelineContext> {
    return assertTimelineContext(await this.fetchJson("/config.json"));
  }
}

export class MotionFileHost extends BaseStudioHost {
  async loadMotion(_request: MotionRequest = {}): Promise<MotionContext> {
    return assertMotionContext(await this.fetchJson("/config.json"));
  }
}

export async function loadStudioHost(
  search = globalThis.location?.search ?? "",
  fetcher: Fetcher = globalThis.fetch.bind(globalThis),
  eventSourceFactory?: EventSourceFactory,
): Promise<StudioHost> {
  const params = new URLSearchParams(search);
  const project = params.get("project");
  const url = project
    ? `/studio/boot.json?project=${encodeURIComponent(project)}`
    : "/studio/boot.json";
  const response = await fetcher(url);
  if (!response.ok) throw new Error(`${url} returned ${response.status}`);
  const raw: unknown = await response.json();
  const boot = assertStudioBoot(raw);
  switch (boot.session.kind) {
    case "project":
      return new ProjectHost(
        boot as StudioBoot & { session: Extract<StudioSession, { kind: "project" }> },
        fetcher,
        eventSourceFactory,
      );
    case "timeline-file":
      return new TimelineFileHost(boot, fetcher, eventSourceFactory);
    case "motion-file":
      return new MotionFileHost(boot, fetcher, eventSourceFactory);
  }
  throw new Error("unreachable Studio session kind");
}

export function assertStudioBoot(value: unknown): StudioBoot {
  if (!isRecord(value) || value.protocolVersion !== STUDIO_HOST_PROTOCOL_VERSION || !isRecord(value.session)) {
    throw new Error(`Studio boot must use protocolVersion ${STUDIO_HOST_PROTOCOL_VERSION} and contain session`);
  }
  const kind = value.session.kind;
  if (!isRecord(value.capabilities) || !isRecord(value.runtime)) {
    throw new Error("Studio boot must contain capabilities and runtime");
  }
  if (kind === "project") {
    if (
      typeof value.session.projectId !== "string"
      || !isRevision(value.session.revision)
      || typeof value.session.token !== "string"
    ) {
      throw new Error("project Studio session is incomplete");
    }
  } else if (kind === "timeline-file") {
    if (typeof value.session.input !== "string") throw new Error("timeline-file input is required");
  } else if (kind === "motion-file") {
    if (typeof value.session.input !== "string" || typeof value.session.generation !== "number") {
      throw new Error("motion-file session is incomplete");
    }
  } else {
    throw new Error(`unknown Studio session kind '${String(kind)}'`);
  }
  return value as unknown as StudioBoot;
}

export function assertMotionContext(value: unknown): MotionContext {
  if (
    !isRecord(value)
    || value.protocolVersion !== STUDIO_HOST_PROTOCOL_VERSION
    || (value.status !== "ok" && value.status !== "error")
    || typeof value.generation !== "number"
    || typeof value.input !== "string"
    || !Array.isArray(value.diagnostics)
    || !value.diagnostics.every(isMotionDiagnostic)
  ) {
    throw new Error(`MotionContext must use protocolVersion ${STUDIO_HOST_PROTOCOL_VERSION} and contain status, generation, input and diagnostics`);
  }
  if (value.status === "error") {
    if (!hasOnlyKeys(value, ["status", "protocolVersion", "generation", "input", "diagnostics"])) {
      throw new Error("error MotionContext must use the closed generated shape");
    }
    return value as unknown as MotionContext;
  }
  if (
    !hasOnlyKeys(value, [
      "status",
      "protocolVersion",
      "generation",
      "input",
      "artifactDigest",
      "artifact",
      "preparedData",
      "dataSource",
      "timing",
      "cueBindings",
      "sourceMap",
      "assets",
      "resourceLocators",
      "shaders",
      "durationFrames",
      "fps",
      "viewport",
      "diagnostics",
      "runtimeBaseUrl",
      "runtimeAssets",
      "fixedPackageManifestJson",
      "timelineJson",
      "timeline",
      "resourceManifestJson",
      "resourceManifest",
      "verifiedBindingBundleJson",
    ])
    || !isSha256Wire(value.artifactDigest)
    || !isMotionArtifact(value.artifact)
    || "controls" in value
    || !isObjectRecord(value.preparedData)
    || (value.dataSource !== null && typeof value.dataSource !== "string")
    || !isRecord(value.timing)
    || !isRecord(value.cueBindings)
    || !Object.values(value.cueBindings).every(isMotionCueBinding)
    || !isMotionSourceMap(value.sourceMap)
    || value.sourceMap.component !== value.artifact.component
    || !Array.isArray(value.assets)
    || !Array.isArray(value.resourceLocators)
    || !Array.isArray(value.shaders)
    || typeof value.durationFrames !== "number"
    || !isRecord(value.fps)
    || !isRecord(value.viewport)
    || !isRecord(value.runtimeAssets)
    || typeof value.fixedPackageManifestJson !== "string"
    || typeof value.timelineJson !== "string"
    || !isRecord(value.timeline)
    || !isRecord(value.timeline.document)
    || typeof value.resourceManifestJson !== "string"
    || !isRecord(value.resourceManifest)
    || !isRecord(value.resourceManifest.entries)
    || typeof value.verifiedBindingBundleJson !== "string"
  ) {
    throw new Error("successful MotionContext must include source-map format 1 and a fixed package");
  }
  return value as unknown as MotionContext;
}

export function motionControls(
  context: Extract<MotionContext, { status: "ok" }>,
): Record<string, unknown> {
  const controls = context.artifact.controls;
  if (!isMotionControlsSchema(controls)) {
    throw new Error("Motion artifact must contain the authoritative controls schema");
  }
  return controls;
}

function assertTimelineContext(value: unknown): TimelineContext {
  if (
    !isRecord(value)
    || "fixedPackageManifestJson" in value
    || "resourceManifestJson" in value
    || "verifiedBindingBundleJson" in value
    || typeof value.timelineJson !== "string"
    || !isTimeline(value.timeline)
    || !isRecord(value.timelineRevision)
    || !isRevision(value.timelineRevision.revision)
    || (value.timelineRevision.parentRevision !== null
      && !isRevision(value.timelineRevision.parentRevision))
    || !isRecord(value.render)
    || !hasOnlyKeys(value.render, [
      "timelineJson",
      "timeline",
      "resourceManifest",
      "fixedPackageManifestJson",
      "resourceManifestJson",
      "verifiedBindingBundleJson",
    ])
    || typeof value.render.timelineJson !== "string"
    || !isRecord(value.render.timeline)
    || !isRecord(value.render.timeline.document)
    || !isRecord(value.runtimeAssets)
    || typeof value.assetBaseUrl !== "string"
  ) {
    throw new Error("Timeline context must contain complete authoring and render projections");
  }
  const hasFixedManifest = value.render.fixedPackageManifestJson !== undefined;
  const hasResourceManifestJson = value.render.resourceManifestJson !== undefined;
  const hasResourceManifest = value.render.resourceManifest !== undefined;
  const hasBindingBundle = value.render.verifiedBindingBundleJson !== undefined;
  if (
    ![hasFixedManifest, hasResourceManifestJson, hasResourceManifest, hasBindingBundle]
      .every((present) => present === hasFixedManifest)
  ) {
    throw new Error(
      "Timeline render context must provide every fixed-package member and manifest together",
    );
  }
  if (hasFixedManifest) {
    if (
      typeof value.render.fixedPackageManifestJson !== "string"
      || value.render.fixedPackageManifestJson.length === 0
      || typeof value.render.resourceManifestJson !== "string"
      || value.render.resourceManifestJson.length === 0
      || !isRecord(value.render.resourceManifest)
      || !isRecord(value.render.resourceManifest.entries)
      || typeof value.render.verifiedBindingBundleJson !== "string"
      || value.render.verifiedBindingBundleJson.length === 0
    ) {
      throw new Error("Timeline render context contains an invalid fixed package");
    }
  }
  if (
    !hasBindingBundle
    && (!isRecord(value.preview) || value.preview.status !== "unavailable" || typeof value.preview.code !== "string")
  ) {
    throw new Error("Timeline context without fixed fulfillment must explicitly mark preview unavailable");
  }
  return value as unknown as TimelineContext;
}

function isRevision(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0;
}

function isTimeline(value: unknown): value is Timeline {
  if (!isRecord(value) || "version" in value || !isRecord(value.canvas) || !isRecord(value.tracks)) {
    return false;
  }
  const bands = new Set(["visual", "audio", "caption", "adjustment"]);
  if (Object.keys(value.tracks).some((key) => !bands.has(key))) return false;
  return Object.values(value.tracks).every((tracks) => (
    Array.isArray(tracks)
    && tracks.every((track) => (
      isRecord(track)
      && !("type" in track)
      && !("trackType" in track)
      && Array.isArray(track.clips)
    ))
  ));
}

/** Project a Motion editor context only after the caller's Product admission has succeeded. */
export function projectMotionContextFromAdmittedPreview(
  config: TimelineContext,
  boot: StudioBoot & { session: Extract<StudioSession, { kind: "project" }> },
  request: MotionRequest,
  authoringTimeline: TimelineDocument = config.render.timeline,
): MotionContext {
  const clipId = request.clipId;
  const motion = record(config.motion);
  const timelineClip = authoringTimeline.document.visual.tracks
    .flatMap((track: { items: TimelineVisualItem[] }) => track.items)
    .find((item: TimelineVisualItem) => item.type === "clip" && item.id === clipId);
  const structure = clipId ? findMotionStructure(motion, clipId) : null;
  const authoring = record(structure?.authoring);
  const sourceMap = record(authoring.sourceMap);
  const motionContent = timelineClip?.type === "clip" && timelineClip.source.type === "motion"
    ? timelineClip.source
    : null;
  const component = motionContent?.component ?? null;
  const missingContext = (missing: string) => motionContextError(
    clipId ?? boot.session.projectId,
    Number(config.generation ?? boot.session.revision),
    "motion-context-missing",
    `Selected project clip is missing ${missing}`,
  );
  if (!clipId) return missingContext("clip id");
  if (!timelineClip) return missingContext("Timeline clip");
  if (!motionContent) return missingContext("Motion source");
  const fixedPackageManifestJson = config.render.fixedPackageManifestJson;
  const resourceManifestJson = config.render.resourceManifestJson;
  const resourceManifest = config.render.resourceManifest;
  const verifiedBindingBundleJson = config.render.verifiedBindingBundleJson;
  if (
    fixedPackageManifestJson === undefined
    || resourceManifestJson === undefined
    || resourceManifest === undefined
    || verifiedBindingBundleJson === undefined
  ) {
    return missingContext("verified fixed-package fulfillment");
  }
  if (!structure) {
    const problems = Array.isArray(motion.problems) ? motion.problems.map(String).join("; ") : "none reported";
    return missingContext(`admitted Motion structure (prepare problems: ${problems})`);
  }
  if (!component) return missingContext("component identity");
  if (sourceMap.version !== MOTION_SOURCE_MAP_VERSION) {
    return missingContext(`source-map format ${MOTION_SOURCE_MAP_VERSION} (received ${String(sourceMap.version)})`);
  }

  const canvas = authoringTimeline.document.canvas;
  const fps = motionFrameRateWire(canvas.fps);
  const assetsById = new Map(arrayOfRecords(config.assets).map((asset) => [String(asset.id), asset]));
  const manifestEntries = record(resourceManifest.entries);
  const artifactDigest = verifiedResourceDigest(manifestEntries, component, "motion-artifact");
  if (!artifactDigest) return missingContext(`verified Motion artifact digest for '${component}'`);
  const artifact = verifiedMotionArtifact(
    verifiedBindingBundleJson,
    component,
    artifactDigest,
  );
  if (!artifact) {
    return missingContext(`Motion artifact for '${component}' bound by the verified package`);
  }
  const resourceLocators = [...assetsById].flatMap(([id, asset]) => (
    typeof asset.url === "string" ? [{ id, url: asset.url }] : []
  ));
  const assets = boundMotionAssets(
    motionContent.resources,
    record(record(artifact.controls).assets),
    assetsById,
    boot.runtime.assetBaseUrl,
  );
  const shaders = arrayOfRecords(motion.shaders).flatMap((shader) => (
    Array.isArray(shader.manifestBytes) && Array.isArray(shader.sourceBytes)
      ? [{
          uri: typeof shader.uri === "string" ? shader.uri : "shader://project",
          manifestBytes: shader.manifestBytes.map(Number),
          sourceBytes: shader.sourceBytes.map(Number),
        }]
      : []
  ));
  // Project Timeline references an artifact whose prepare data is already baked. Raw authoring
  // data is intentionally not reconstructed from the runtime clip contract.
  const preparedData = {};
  const timing = resolvedMotionTiming(artifact, authoring);
  const cueBindings = resolvedMotionCues(authoring, motionContent);
  if (!cueBindings) return missingContext("resolved Motion cue windows");
  const totalFrames = motionTotalFrames(authoring);
  return assertMotionContext({
    status: "ok",
    protocolVersion: STUDIO_HOST_PROTOCOL_VERSION,
    generation: Number(config.generation ?? boot.session.revision),
    input: typeof sourceMap.entry === "string" ? sourceMap.entry : `components/${component}.tsx`,
    artifactDigest,
    artifact,
    preparedData,
    dataSource: null,
    timing,
    cueBindings,
    sourceMap,
    assets,
    resourceLocators,
    shaders,
    durationFrames: totalFrames,
    fps,
    viewport: { width: canvas.width, height: canvas.height },
    diagnostics: (Array.isArray(motion.problems) ? motion.problems : []).map((problem) => ({
      class: "prepare",
      code: "motion-project-prepare",
      message: String(problem),
    })),
    runtimeBaseUrl: "/",
    runtimeAssets: record(config.runtimeAssets),
    fixedPackageManifestJson,
    timelineJson: config.render.timelineJson,
    timeline: config.render.timeline,
    resourceManifestJson,
    resourceManifest,
    verifiedBindingBundleJson,
  });
}

function motionContextError(input: string, generation: number, code: string, message: string): MotionContext {
  return {
    status: "error",
    protocolVersion: STUDIO_HOST_PROTOCOL_VERSION,
    generation,
    input,
    diagnostics: [{ class: "session", code, message }],
  };
}

function boundMotionAssets(
  bindings: Record<string, unknown>,
  controls: Record<string, unknown>,
  assetsById: Map<string, Record<string, unknown>>,
  assetBaseUrl: string,
): Array<{ name: string; kind: string; url: string }> {
  return Object.entries(bindings).flatMap(([name, reference]) => {
    const assetId = typeof reference === "string" && reference.startsWith("asset:")
      ? reference.slice("asset:".length)
      : null;
    const asset = assetId ? assetsById.get(assetId) : undefined;
    const kind = String(record(controls[name]).kind ?? "");
    return asset && typeof asset.url === "string"
      ? [{ name, kind, url: resolveHostedAssetUrl(asset.url, assetBaseUrl) }]
      : [];
  });
}

function resolvedMotionTiming(
  artifact: Record<string, unknown>,
  authoring: Record<string, unknown>,
): { enterFrames: number; exitFrames: number } {
  const prepared = record(authoring.timing);
  const controls = record(record(artifact.controls).timing);
  const enter = record(controls.enterFrames);
  const exit = record(controls.exitFrames);
  return {
    enterFrames: Number(prepared.enterFrames ?? enter.default ?? 0),
    exitFrames: Number(prepared.exitFrames ?? exit.default ?? 0),
  };
}

function resolvedMotionCues(
  authoring: Record<string, unknown>,
  source: TimelineMotionSource,
): MotionContextOk["cueBindings"] | null {
  const resolved = record(authoring.cues);
  const result: MotionContextOk["cueBindings"] = {};
  for (const [name, binding] of Object.entries(
    source.cues as Record<string, { type: string }>,
  )) {
    const prepared = record(resolved[name]);
    const startFrame = Number(prepared.startFrame);
    const endFrame = Number(prepared.endFrame);
    const enterFrames = Number(prepared.enterFrames ?? 0);
    const exitFrames = Number(prepared.exitFrames ?? 0);
    if (![startFrame, endFrame, enterFrames, exitFrames].every(isNonNegativeSafeInteger)) {
      return null;
    }
    if (binding.type !== "source-range") return null;
    result[name] = {
      type: "sourceRange",
      startFrame,
      endFrame,
      enterFrames,
      exitFrames,
    };
  }
  return result;
}

function isMotionCueBinding(value: unknown): boolean {
  if (!isRecord(value) || value.type !== "sourceRange") return false;
  const fields = ["startFrame", "endFrame", "enterFrames", "exitFrames"] as const;
  return fields.every((field) => isNonNegativeSafeInteger(value[field]))
    && Object.keys(value).every((key) => key === "type" || fields.includes(key as typeof fields[number]));
}

function isNonNegativeSafeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function motionTotalFrames(authoring: Record<string, unknown>): number {
  return Math.max(1, Number(authoring.totalFrames ?? 1));
}

function resolveHostedAssetUrl(url: string, assetBaseUrl: string): string {
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(url) || url.startsWith("/")) return url;
  const base = assetBaseUrl.endsWith("/") ? assetBaseUrl : `${assetBaseUrl}/`;
  return `${base}${url.split("/").map(encodeURIComponent).join("/")}`;
}

function verifiedResourceDigest(
  entries: Record<string, unknown>,
  resourceId: string,
  kind: string,
): string | null {
  const entry = record(entries[resourceId]);
  return entry.kind === kind && isSha256Wire(entry.digest) ? entry.digest : null;
}

function verifiedMotionArtifact(
  verifiedBindingBundleJson: string,
  resourceId: string,
  digest: string,
): Record<string, unknown> | null {
  let value: unknown;
  try {
    value = JSON.parse(verifiedBindingBundleJson) as unknown;
  } catch {
    return null;
  }
  if (!isObjectRecord(value)) return null;
  const bindings = value.bindings;
  if (!isObjectRecord(bindings)) return null;
  const binding = bindings[resourceId];
  if (!isObjectRecord(binding) || binding.digest !== digest) return null;
  const facts = binding.facts;
  if (!isObjectRecord(facts) || facts.kind !== "motion-artifact") return null;
  return isMotionArtifact(facts.artifact) ? facts.artifact : null;
}

function isSha256Wire(value: unknown): value is string {
  return typeof value === "string" && /^sha256:[0-9a-f]{64}$/.test(value);
}

function isMotionSourceMap(value: unknown): value is MotionContextOk["sourceMap"] {
  if (
    !isObjectRecord(value)
    || !hasOnlyKeys(value, [
      "version",
      "component",
      "entry",
      "closureDigest",
      "modules",
      "controlsSourcePath",
      "controls",
      "nodes",
      "exprs",
      "objects",
    ])
  ) {
    return false;
  }
  return value.version === 1
    && typeof value.component === "string"
    && typeof value.entry === "string"
    && isSha256Wire(value.closureDigest)
    && Array.isArray(value.modules)
    && value.modules.every(isMotionSourceModule)
    && (value.controlsSourcePath === undefined || typeof value.controlsSourcePath === "string")
    && (value.controls === undefined || isObjectRecord(value.controls))
    && Array.isArray(value.nodes)
    && value.nodes.every(isObjectRecord)
    && Array.isArray(value.exprs)
    && value.exprs.every(isObjectRecord)
    && Array.isArray(value.objects)
    && value.objects.every(isObjectRecord);
}

function isMotionSourceModule(value: unknown): boolean {
  return isObjectRecord(value)
    && hasOnlyKeys(value, ["path", "sourceDigest", "normalizedAstDigest"])
    && typeof value.path === "string"
    && isSha256Wire(value.sourceDigest)
    && isSha256Wire(value.normalizedAstDigest);
}

function isMotionDiagnostic(value: unknown): boolean {
  if (!isRecord(value)
    || typeof value.class !== "string"
    || typeof value.code !== "string"
    || typeof value.message !== "string"
    || (value.sourcePath !== undefined && typeof value.sourcePath !== "string")) {
    return false;
  }
  const span = value.span;
  if (span === undefined) return true;
  if (!isRecord(span)) return false;
  return ["start", "end", "line", "column"].every((field) => isNonNegativeSafeInteger(span[field]))
    && Object.keys(span).every((key) => ["start", "end", "line", "column"].includes(key));
}

function motionFrameRateWire(value: unknown): { num: number; den: number } {
  if (typeof value !== "string") throw new Error("canonical fps must be an exact rational string");
  const match = /^([1-9][0-9]*)\/([1-9][0-9]*)$/.exec(value);
  if (!match) throw new Error(`canonical fps '${value}' is invalid`);
  return { num: Number(match[1]), den: Number(match[2]) };
}

function arrayOfRecords(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.filter(isRecord) : [];
}

function record(value: unknown): Record<string, unknown> {
  return isRecord(value) ? value : {};
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function hasOnlyKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const allowed = new Set(keys);
  return Object.keys(value).every((key) => allowed.has(key));
}

function isObjectRecord(value: unknown): value is Record<string, unknown> {
  return isRecord(value) && !Array.isArray(value);
}

function isMotionArtifact(value: unknown): value is Record<string, unknown> {
  return isObjectRecord(value)
    && value.formatVersion === 1
    && typeof value.component === "string"
    && value.component.length > 0
    && isMotionControlsSchema(value.controls);
}

function isMotionControlsSchema(value: unknown): value is Record<string, unknown> {
  if (!isObjectRecord(value)) return false;
  const timing = value.timing;
  const camera = value.camera;
  return isObjectRecord(value.props)
    && isObjectRecord(value.data)
    && isObjectRecord(timing)
    && isObjectRecord(timing.enterFrames)
    && isObjectRecord(timing.holdCycleFrames)
    && isObjectRecord(timing.exitFrames)
    && isObjectRecord(value.cues)
    && isObjectRecord(value.assets)
    && isObjectRecord(camera)
    && isObjectRecord(camera.values);
}
