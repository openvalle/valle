// Product frame-planning worker contract.
//
// The worker owns the complete CPU-side ProductEngine pipeline for a frame. Browser resources,
// CanvasKit objects and presentation deliberately stay on the main thread because they are tied to
// DOM/WebGL ownership. Every message is RenderId- and epoch-scoped; stale work is discarded rather
// than being rebound to a different product state.

export const PRODUCT_FRAME_WORKER_PROTOCOL_VERSION = 1 as const;

export interface ProductEngineWire {
  open_fixed_package(
    fixedPackageManifestJson: string,
    timelineJson: string,
    resourceManifestJson: string,
    verifiedBindingBundleJson: string,
  ): string;
  frame_at_seconds(renderId: string, seconds: number): bigint;
  sample_at_seconds(renderId: string, seconds: number): bigint;
  audio_program_json(renderId: string): string;
  audio_sample_json(renderId: string, sample: bigint): string;
  audio_block_json(renderId: string, startSample: bigint, endSample: bigint): string;
  compiled_execution_resources_json(renderId: string): string;
  compiled_resource_bytes(
    renderId: string,
    kind: "font-bytes" | "runtime-shader",
    contentDigest: string,
    abiDigest?: string | null,
  ): Uint8Array;
  evaluate_prepare_preview(
    renderId: string,
    frame: bigint,
    width: number,
    height: number,
    transparent: boolean,
  ): number;
  resource_requests(ticket: number): Uint8Array;
  frame_inspection_json(ticket: number): string;
  lower_canvas_kit(ticket: number, maxSurfaceBytes: bigint, maxFrameBytes: bigint): void;
  template_cache_hit(ticket: number): boolean;
  bind(ticket: number, generation: bigint): void;
  plan_template_hash(ticket: number): string;
  plan_template_bytes(ticket: number): Uint8Array;
  binding_bytes(ticket: number): Uint8Array;
  bound_schedule_bytes(ticket: number): Uint8Array;
  release_ticket(ticket: number): boolean;
  pack_motion_glass_uniforms(
    programJson: string,
    ownerToDevice: Float64Array,
  ): Float32Array;
  pack_motion_glass_foreground_uniforms(
    programJson: string,
    ownerToDevice: Float64Array,
  ): Float32Array;
  free(): void;
}

export interface ProductWasmModule {
  default(wasmUrl: string): Promise<void>;
  canonicalize_timeline_document(timelineJson: string): string;
  ProductEngine: new () => ProductEngineWire;
}

export interface ProductEngineBootstrap {
  wasmModuleUrl: string;
  wasmUrl: string;
  fixedPackageManifestJson: string;
  timelineJson: string;
  resourceManifestJson: string;
  verifiedBindingBundleJson: string;
  expectedRenderId: string;
}

export interface ProductRenderReceipt {
  renderId: string;
  canvasWidth: number;
  canvasHeight: number;
  frameRate: string;
  frameCount: number;
  sampleRate: number;
  sampleCount: number;
}

export interface ProductFrameRequest {
  key: string;
  renderId: string;
  epoch: number;
  frame: number;
  width: number;
  height: number;
  transparent: boolean;
  generation: bigint;
  maxSurfaceBytes: bigint;
  maxFrameBytes: bigint;
}

interface WorkerMessageBase {
  protocolVersion: typeof PRODUCT_FRAME_WORKER_PROTOCOL_VERSION;
  id: string;
}

export type ProductFrameWorkerRequest =
  | (WorkerMessageBase & { type: "configure"; bootstrap: ProductEngineBootstrap })
  | (WorkerMessageBase & {
      type: "prepareBatch";
      jobs: Array<{ id: string; request: ProductFrameRequest }>;
    });

export interface ProductFrameRequestStage {
  key: string;
  renderId: string;
  epoch: number;
  frame: number;
  generation: bigint;
  requestPacket: Uint8Array;
  inspectionJson: string;
  evaluatePrepareMs: number;
  requestInspectMs: number;
  workerTurnGapMs: number;
  previousReleaseMs: number;
}

export interface ProductFrameReadyStage {
  key: string;
  renderId: string;
  epoch: number;
  frame: number;
  generation: bigint;
  templateHash: string;
  planPacket: Uint8Array;
  planPacketTransferred: boolean;
  bindingPacket: Uint8Array;
  schedulePacket: Uint8Array;
  templateCacheHit: boolean;
  lowerMs: number;
  bindPacketsMs: number;
  totalWorkerMs: number;
}

export interface ProductFrameWorkerReadyStage extends Omit<
  ProductFrameReadyStage,
  "planPacket" | "planPacketTransferred"
> {
  planPacket: Uint8Array | null;
}

export type ProductFrameWorkerFatalPhase = "protocol" | "prepareBatch";

export type ProductFrameWorkerFatalResponse = WorkerMessageBase & {
  type: "fatal";
  phase: ProductFrameWorkerFatalPhase;
  error: string;
};

export type ProductFrameWorkerResponse =
  | (WorkerMessageBase & { type: "configured"; renderId?: string; error?: string })
  | (WorkerMessageBase & { type: "requests"; stage: ProductFrameRequestStage })
  | (WorkerMessageBase & { type: "ready"; stage: ProductFrameWorkerReadyStage })
  | (WorkerMessageBase & {
      type: "requestsBatch";
      entries: Array<{ id: string; stage: ProductFrameRequestStage }>;
    })
  | (WorkerMessageBase & {
      type: "readyBatch";
      entries: Array<{ id: string; stage: ProductFrameWorkerReadyStage }>;
    })
  | (WorkerMessageBase & {
      type: "taskError";
      key: string;
      renderId: string;
      epoch: number;
      error: string;
    })
  | ProductFrameWorkerFatalResponse;

export function productFrameWorkerFatalResponse(
  request: unknown,
  phase: ProductFrameWorkerFatalPhase,
  error: unknown,
): ProductFrameWorkerFatalResponse {
  const raw = typeof request === "object" && request !== null && !Array.isArray(request)
    ? request as Record<string, unknown>
    : {};
  return {
    protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
    type: "fatal",
    id: typeof raw.id === "string" && raw.id.length > 0 ? raw.id : "protocol",
    phase,
    error: error instanceof Error ? error.stack ?? error.message : String(error),
  };
}

export function emptyProductEngineBootstrap(
  wasmModuleUrl: string,
  wasmUrl: string,
  fixedPackageManifestJson: string,
  timelineJson: string,
  resourceManifestJson: string,
  verifiedBindingBundleJson: string,
): ProductEngineBootstrap {
  return {
    wasmModuleUrl,
    wasmUrl,
    fixedPackageManifestJson,
    timelineJson,
    resourceManifestJson,
    verifiedBindingBundleJson,
    expectedRenderId: "",
  };
}

export function applyProductEngineBootstrap(
  engine: ProductEngineWire,
  bootstrap: ProductEngineBootstrap,
): string {
  const receipt = parseProductRenderReceipt(engine.open_fixed_package(
    bootstrap.fixedPackageManifestJson,
    bootstrap.timelineJson,
    bootstrap.resourceManifestJson,
    bootstrap.verifiedBindingBundleJson,
  ));
  if (
    bootstrap.expectedRenderId
    && receipt.renderId !== bootstrap.expectedRenderId
  ) {
    throw new Error(
      `ProductEngine worker RenderId drift: expected ${bootstrap.expectedRenderId}, got ${receipt.renderId}`,
    );
  }
  return receipt.renderId;
}

export function parseProductRenderReceipt(json: string): ProductRenderReceipt {
  const value = requireRecord(JSON.parse(json) as unknown, "Product render receipt");
  const fields = [
    "renderId",
    "canvasWidth",
    "canvasHeight",
    "frameRate",
    "frameCount",
    "sampleRate",
    "sampleCount",
  ] as const;
  requireExactFields(value, fields, "Product render receipt");
  for (const field of ["renderId", "frameRate"] as const) {
    if (typeof value[field] !== "string" || value[field].length === 0) {
      throw new TypeError(`Product render receipt ${field} is required`);
    }
  }
  requireRenderId(value.renderId, "Product render receipt renderId");
  for (const field of [
    "canvasWidth",
    "canvasHeight",
    "frameCount",
    "sampleRate",
    "sampleCount",
  ] as const) {
    if (!Number.isSafeInteger(value[field]) || Number(value[field]) <= 0) {
      throw new TypeError(`Product render receipt ${field} must be a positive safe integer`);
    }
  }
  return value as unknown as ProductRenderReceipt;
}

export function assertProductFrameWorkerRequest(
  value: unknown,
): asserts value is ProductFrameWorkerRequest {
  const message = requireRecord(value, "Product frame worker request");
  if (message.protocolVersion !== PRODUCT_FRAME_WORKER_PROTOCOL_VERSION) {
    throw new TypeError(`unsupported Product frame worker protocol '${String(message.protocolVersion)}'`);
  }
  if (typeof message.id !== "string" || message.id.length === 0) {
    throw new TypeError("Product frame worker request id is required");
  }
  if (message.type !== "configure" && message.type !== "prepareBatch") {
    throw new TypeError(`unknown Product frame worker request '${String(message.type)}'`);
  }
  if (message.type === "configure") {
    const bootstrap = requireRecord(message.bootstrap, "Product frame worker bootstrap");
    const fields = [
      "wasmModuleUrl",
      "wasmUrl",
      "fixedPackageManifestJson",
      "timelineJson",
      "resourceManifestJson",
      "verifiedBindingBundleJson",
      "expectedRenderId",
    ] as const;
    requireExactFields(bootstrap, fields, "Product frame worker bootstrap");
    for (const field of fields.slice(0, -1)) {
      if (typeof bootstrap[field] !== "string" || bootstrap[field].length === 0) {
        throw new TypeError(`Product frame worker bootstrap ${field} is required`);
      }
    }
    requireRenderId(
      bootstrap.expectedRenderId,
      "Product frame worker bootstrap expectedRenderId",
    );
  } else {
    if (!Array.isArray(message.jobs) || message.jobs.length === 0) {
      throw new TypeError("Product frame worker batch must contain at least one job");
    }
    for (const value of message.jobs) {
      const job = requireRecord(value, "Product frame worker job");
      if (typeof job.id !== "string" || job.id.length === 0) {
        throw new TypeError("Product frame worker job id is required");
      }
      const request = requireRecord(job.request, "Product frame request");
      requireRenderId(request.renderId, "Product frame request renderId");
    }
  }
}

export function assertProductFrameWorkerResponse(
  value: unknown,
): asserts value is ProductFrameWorkerResponse {
  const message = requireRecord(value, "Product frame worker response");
  if (message.protocolVersion !== PRODUCT_FRAME_WORKER_PROTOCOL_VERSION) {
    throw new TypeError(`unsupported Product frame worker protocol '${String(message.protocolVersion)}'`);
  }
  if (typeof message.id !== "string" || message.id.length === 0) {
    throw new TypeError("Product frame worker response id is required");
  }
  if (![
    "configured", "requests", "ready", "requestsBatch", "readyBatch", "taskError", "fatal",
  ].includes(String(message.type))) {
    throw new TypeError(`unknown Product frame worker response '${String(message.type)}'`);
  }
  if (message.type === "configured") {
    if (typeof message.error !== "string") {
      requireRenderId(
        message.renderId,
        "configured Product frame worker renderId",
      );
    }
    return;
  }
  if (message.type === "fatal") {
    if (message.phase !== "protocol" && message.phase !== "prepareBatch") {
      throw new TypeError(`unknown Product frame worker fatal phase '${String(message.phase)}'`);
    }
    if (typeof message.error !== "string" || message.error.length === 0) {
      throw new TypeError("Product frame worker fatal error is required");
    }
    return;
  }
  if (message.type === "requests" || message.type === "ready") {
    const stage = requireRecord(message.stage, `Product frame ${message.type} stage`);
    requireRenderId(stage.renderId, `Product frame ${message.type} renderId`);
    return;
  }
  if (message.type === "requestsBatch" || message.type === "readyBatch") {
    if (!Array.isArray(message.entries) || message.entries.length === 0) {
      throw new TypeError(`Product frame ${message.type} must contain entries`);
    }
    for (const value of message.entries) {
      const entry = requireRecord(value, `Product frame ${message.type} entry`);
      const stage = requireRecord(entry.stage, `Product frame ${message.type} stage`);
      requireRenderId(stage.renderId, `Product frame ${message.type} renderId`);
    }
    return;
  }
  requireRenderId(message.renderId, "Product frame task error renderId");
}

function requireRenderId(value: unknown, label: string): string {
  if (typeof value !== "string" || !/^sha256:[0-9a-f]{64}$/.test(value)) {
    throw new TypeError(`${label} must be a sha256 RenderId`);
  }
  return value;
}

function requireRecord(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TypeError(`${label} must be an object`);
  }
  return value as Record<string, unknown>;
}

function requireExactFields(
  value: Record<string, unknown>,
  expected: readonly string[],
  label: string,
): void {
  const expectedFields = new Set(expected);
  for (const field of Object.keys(value)) {
    if (!expectedFields.has(field)) {
      throw new TypeError(`${label} contains unknown field ${field}`);
    }
  }
  for (const field of expected) {
    if (!(field in value)) {
      throw new TypeError(`${label} ${field} is required`);
    }
  }
}
