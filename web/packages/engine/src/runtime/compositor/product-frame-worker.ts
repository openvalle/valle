import {
  PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
  PRODUCT_FRAME_TEMPLATE_CACHE_SIZE,
  parseProductRenderReceipt,
  assertProductFrameWorkerRequest,
  productFrameWorkerFatalResponse,
  type ProductEngineWire,
  type ProductFrameRequest,
  type ProductFrameRequestStage,
  type ProductFrameWorkerReadyStage,
  type ProductFrameWorkerRequest,
  type ProductFrameWorkerResponse,
  type ProductWasmModule,
  type MotionKernelJob,
  type MotionKernelDefinition,
} from "./frame-protocol.ts";
import { decodePackedAbi, RESOURCE_REQUESTS_ABI } from "../../abi/packed.ts";
import { supportsScene3dGpu, scene3dHasExternalResources } from "./scene3d-gpu-request.ts";
import type { Scene3dGpuRequest } from "../../executor/webgl/scene3d.ts";
import { executeRuntimeCall } from "./runtime-operations.ts";
import { installKernelWorker, type Scene3dKernelInput } from "./kernel-worker.ts";

interface ProductWorkerScope {
  onmessage: ((event: MessageEvent<unknown>) => void) | null;
  postMessage(message: ProductFrameWorkerResponse, transfer?: Transferable[]): void;
}

const scope = globalThis as unknown as ProductWorkerScope;
let engine: ProductEngineWire | null = null;
let renderId: string | null = null;
let messageChain = Promise.resolve();
const sentTemplateHashes = new Set<string>();
let lastReadyAt: number | null = null;
let lastReleaseMs = 0;
const motionDefinitionsSent = new Map<string, number>();
// Dispatch can still prefetch multiple jobs, but publish each frame immediately. Waiting for
// eight evaluate/prepare calls delays both resource fulfillment and the first visible frame.
const RESULT_BATCH_FRAMES = 1;

interface StartedFrame {
  id: string;
  request: ProductFrameRequest;
  ticket: number;
  requestStage: ProductFrameRequestStage;
}

if (new URL(globalThis.location.href).searchParams.get("valle-kernel") === "1")
  installKernelWorker();
else
  scope.onmessage = (event) => {
    messageChain = messageChain
      .then(() => handleIncomingMessage(event.data))
      .catch((error) => {
        // A response-delivery failure cannot be assigned to one task. Try once to fail the worker
        // explicitly; if the message channel itself is broken, the browser's worker error owns it.
        try {
          scope.postMessage(productFrameWorkerFatalResponse(event.data, "protocol", error));
        } catch {
          // Nothing else can be delivered through this worker channel.
        }
      });
  };

async function handleIncomingMessage(value: unknown): Promise<void> {
  try {
    assertProductFrameWorkerRequest(value);
  } catch (error) {
    scope.postMessage(productFrameWorkerFatalResponse(value, "protocol", error));
    return;
  }

  try {
    await handleMessage(value);
  } catch (error) {
    if (value.type === "configure") {
      scope.postMessage({
        protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
        type: "configured",
        id: value.id,
        error: errorMessage(error),
      });
      return;
    }
    scope.postMessage(productFrameWorkerFatalResponse(value, "prepareBatch", error));
  }
}

async function handleMessage(message: ProductFrameWorkerRequest): Promise<void> {
  if (message.type === "configure") {
    engine?.free();
    engine = null;
    renderId = null;
    sentTemplateHashes.clear();
    motionDefinitionsSent.clear();
    lastReadyAt = null;
    lastReleaseMs = 0;
    const wasm = (await import(message.bootstrap.wasmModuleUrl)) as ProductWasmModule;
    await wasm.default({ module_or_path: message.bootstrap.wasmUrl });
    const canonicalTimelineJson = wasm.canonicalize_timeline_document(
      message.bootstrap.timelineJson,
    );
    const bootstrap = {
      ...message.bootstrap,
      timelineJson: canonicalTimelineJson,
    };
    const next = new wasm.ProductEngine();
    next.set_delivery_output?.(bootstrap.deliveryOutput === true);
    try {
      const receipt = parseProductRenderReceipt(
        next.open_fixed_package(
          bootstrap.fixedPackageManifestJson,
          bootstrap.timelineJson,
          bootstrap.resourceManifestJson,
          bootstrap.verifiedBindingBundleJson,
          bootstrap.resourceBytes ?? new Uint8Array(),
        ),
      );
      const openedRenderId = receipt.renderId;
      if (bootstrap.expectedRenderId && bootstrap.expectedRenderId !== openedRenderId)
        throw new Error("Product worker RenderId drift");
      const resources = JSON.parse(next.compiled_execution_resources_json(openedRenderId)).map(
        (resource: any) => ({
          ...resource,
          bytes: next.compiled_resource_bytes(
            openedRenderId,
            resource.kind,
            resource.contentDigest,
            resource.abiDigest,
          ),
        }),
      );
      const snapshot = {
        receipt,
        resources,
        audioProgramJson: next.audio_program_json(openedRenderId),
      };
      next.set_realtime_geometry?.(false);
      engine = next;
      renderId = openedRenderId;
      scope.postMessage({
        protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
        type: "configured",
        id: message.id,
        renderId: openedRenderId,
        snapshot,
      });
    } catch (error) {
      next.free();
      throw error;
    }
    return;
  }

  if (message.type === "call") {
    try {
      if (!engine || renderId !== message.renderId)
        throw new Error("Product runtime call RenderId drift");
      const result = executeRuntimeCall(engine, message.method, message.args);
      scope.postMessage(
        {
          protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
          type: "callResult",
          id: message.id,
          renderId,
          result,
        },
        ArrayBuffer.isView(result) ? [result.buffer as ArrayBuffer] : [],
      );
    } catch (error) {
      scope.postMessage({
        protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
        type: "callResult",
        id: message.id,
        renderId: message.renderId,
        error: errorMessage(error),
      });
    }
    return;
  }

  for (let offset = 0; offset < message.jobs.length; offset += RESULT_BATCH_FRAMES) {
    const jobs = message.jobs.slice(offset, offset + RESULT_BATCH_FRAMES);
    engine?.set_realtime_geometry?.(jobs[0]?.request.realtime ?? false);
    engine?.set_delivery_output?.(jobs[0]?.request.deliveryOutput ?? false);
    const started: StartedFrame[] = [];
    for (const [index, job] of jobs.entries()) {
      try {
        if (engine?.motion_kernel_available?.() && job.motionResults === undefined) {
          if (job.request.renderId !== renderId) throw new Error("Motion capture RenderId drift");
          const r = job.request;
          const work = JSON.parse(
            engine.capture_motion_jobs_preview!(
              r.renderId,
              BigInt(r.frame),
              r.width,
              r.height,
              r.transparent,
            ),
          ) as MotionKernelJob[];
          const definitions: Record<string, MotionKernelDefinition> = {};
          for (const input of work) {
            if (!motionDefinitionsSent.has(input.definitionId))
              definitions[input.definitionId] = {
                json: engine.motion_kernel_definition!(input.definitionId),
                resourceBytes: engine.motion_kernel_definition_bytes!(input.definitionId),
              };
            const length =
              (definitions[input.definitionId]
                ? definitions[input.definitionId]!.json.length +
                  definitions[input.definitionId]!.resourceBytes.byteLength
                : undefined) ?? motionDefinitionsSent.get(input.definitionId)!;
            motionDefinitionsSent.delete(input.definitionId);
            motionDefinitionsSent.set(input.definitionId, length);
          }
          while (
            motionDefinitionsSent.size > 4 ||
            [...motionDefinitionsSent.values()].reduce((sum, bytes) => sum + bytes, 0) >
              64 * 1024 ** 2
          ) {
            motionDefinitionsSent.delete(motionDefinitionsSent.keys().next().value!);
          }
          if (work.length) {
            scope.postMessage(
              {
                protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
                type: "motionWork",
                id: job.id,
                request: r,
                jobs: work,
                definitions,
              },
              Object.values(definitions).map(
                (definition) => definition.resourceBytes.buffer as ArrayBuffer,
              ),
            );
            continue;
          }
        }
        started.push(await startFrame(job.id, job.request, index === 0, job.motionResults));
      } catch (error) {
        postTaskError(job.id, job.request, error);
      }
    }
    if (started.length === 0) continue;
    const bundledRequests = bundleByteArrays(
      started.map((frame) => frame.requestStage.requestPacket),
    );
    scope.postMessage(
      {
        protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
        type: "requestsBatch",
        id: `requests-${message.id}-${offset}`,
        entries: started.map((frame, index) => ({
          id: frame.id,
          stage: {
            ...frame.requestStage,
            requestPacket: bundledRequests.views[index]!,
          },
        })),
      },
      [
        bundledRequests.buffer.buffer,
        ...started.flatMap((f) =>
          Object.values(f.requestStage.gpuScene3d ?? {}).map((v) => v.buffer as ArrayBuffer),
        ),
      ],
    );

    const ready: Array<{ id: string; stage: ProductFrameWorkerReadyStage }> = [];
    for (const frame of started) {
      try {
        ready.push({ id: frame.id, stage: finishFrame(frame) });
      } catch (error) {
        postTaskError(frame.id, frame.request, error);
      }
    }
    if (ready.length === 0) continue;
    // Per-frame packets share one buffer. A plan packet outlives its frame in the main thread's
    // template caches, so it keeps the exact buffer the engine returned instead of pinning the
    // whole batch.
    const chunks: Uint8Array[] = [];
    const packetIndexes: Array<{ binding: number; schedule: number }> = [];
    for (const entry of ready) {
      const binding = chunks.push(entry.stage.bindingPacket) - 1;
      const schedule = chunks.push(entry.stage.schedulePacket) - 1;
      packetIndexes.push({ binding, schedule });
    }
    const bundledReady = bundleByteArrays(chunks);
    lastReadyAt = performance.now();
    scope.postMessage(
      {
        protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
        type: "readyBatch",
        id: `ready-${message.id}-${offset}`,
        entries: ready.map((entry, index) => {
          const packets = packetIndexes[index]!;
          return {
            id: entry.id,
            stage: {
              ...entry.stage,
              bindingPacket: bundledReady.views[packets.binding]!,
              schedulePacket: bundledReady.views[packets.schedule]!,
            },
          };
        }),
      },
      [
        bundledReady.buffer.buffer,
        ...ready.flatMap((entry) =>
          entry.stage.planPacket === null ? [] : [entry.stage.planPacket.buffer as ArrayBuffer],
        ),
      ],
    );
  }
}

async function startFrame(
  messageId: string,
  request: ProductFrameRequest,
  measureTurnGap: boolean,
  motionResults?: Uint8Array,
): Promise<StartedFrame> {
  const active = engine;
  const activeRenderId = renderId;
  if (!active || !activeRenderId) throw new Error("Product frame worker is not configured");
  if (request.renderId !== activeRenderId) {
    throw new Error(
      `Product frame request RenderId drift: expected ${activeRenderId}, got ${request.renderId}`,
    );
  }
  const workerTurnGapMs =
    !measureTurnGap || lastReadyAt === null ? 0 : performance.now() - lastReadyAt;
  const evaluateStarted = performance.now();
  const ticket =
    motionResults !== undefined
      ? active.evaluate_prepare_motion_preview!(
          activeRenderId,
          BigInt(request.frame),
          request.width,
          request.height,
          request.transparent,
          motionResults,
        )
      : active.evaluate_prepare_preview(
          activeRenderId,
          BigInt(request.frame),
          request.width,
          request.height,
          request.transparent,
        );
  const evaluatePrepareMs = performance.now() - evaluateStarted;
  try {
    const inspectStarted = performance.now();
    const requestPacket = active.resource_requests(ticket);
    const inspectionJson = active.frame_inspection_json(ticket);
    const gpuJobs: Record<string, Scene3dKernelInput> = {};
    if (request.gpuScene3d && active.scene3d_gpu_raster_input) {
      const requests = (await decodePackedAbi(requestPacket, RESOURCE_REQUESTS_ABI)) as Record<
        string,
        any
      >[];
      for (const request of requests) {
        if (request.expected.kind !== "scene3d") continue;
        const content = request.key.content;
        if (gpuJobs[content]) continue;
        const canonical = request.payload.canonical_request as string;
        const value = JSON.parse(canonical) as Scene3dGpuRequest;
        if (!supportsScene3dGpu(value) || scene3dHasExternalResources(value)) continue;
        gpuJobs[content] = {
          content,
          topology: request.key.interpretation.topology_digest,
          canonical,
          width: value.width,
          height: value.height,
        };
      }
    }
    const requestInspectMs = performance.now() - inspectStarted;
    return {
      id: messageId,
      request,
      ticket,
      requestStage: {
        key: request.key,
        renderId: activeRenderId,
        epoch: request.epoch,
        frame: request.frame,
        generation: request.generation,
        requestPacket,
        inspectionJson,
        ...(Object.keys(gpuJobs).length ? { gpuScene3dJobs: gpuJobs } : {}),
        ...(request.trace
          ? {
              workerTimeOriginMs: performance.timeOrigin,
              evaluateStartedMs: evaluateStarted,
              inspectStartedMs: inspectStarted,
            }
          : {}),
        evaluatePrepareMs,
        requestInspectMs,
        workerTurnGapMs,
        previousReleaseMs: measureTurnGap ? lastReleaseMs : 0,
      },
    };
  } catch (error) {
    active.release_ticket(ticket);
    throw error;
  }
}

function finishFrame(frame: StartedFrame): ProductFrameWorkerReadyStage {
  const { request, ticket } = frame;
  const active = engine;
  const activeRenderId = renderId;
  if (!active || !activeRenderId) throw new Error("Product frame worker is not configured");
  if (request.renderId !== activeRenderId) {
    throw new Error(
      `Product frame finish RenderId drift: expected ${activeRenderId}, got ${request.renderId}`,
    );
  }
  try {
    const lowerStarted = performance.now();
    active.lower_canvas_kit(ticket, request.maxSurfaceBytes, request.maxFrameBytes);
    const lowerMs = performance.now() - lowerStarted;
    const templateCacheHit = active.template_cache_hit(ticket);
    const templateHash = active.plan_template_hash(ticket);
    const templateIdentity = `${activeRenderId}:${templateHash}`;
    const bindStarted = performance.now();
    active.bind(ticket, request.generation);
    const transmitPlan = !sentTemplateHashes.has(templateIdentity);
    const planPacket = transmitPlan ? active.plan_template_bytes(ticket) : null;
    const bindingPacket = active.binding_bytes(ticket);
    const schedulePacket = active.bound_schedule_bytes(ticket);
    sentTemplateHashes.delete(templateIdentity);
    sentTemplateHashes.add(templateIdentity);
    if (sentTemplateHashes.size > PRODUCT_FRAME_TEMPLATE_CACHE_SIZE) {
      sentTemplateHashes.delete(sentTemplateHashes.values().next().value!);
    }
    const bindPacketsMs = performance.now() - bindStarted;
    return {
      key: request.key,
      renderId: activeRenderId,
      epoch: request.epoch,
      frame: request.frame,
      generation: request.generation,
      templateHash,
      planPacket,
      bindingPacket,
      schedulePacket,
      templateCacheHit,
      ...(request.trace
        ? {
            workerTimeOriginMs: performance.timeOrigin,
            lowerStartedMs: lowerStarted,
            bindStartedMs: bindStarted,
          }
        : {}),
      lowerMs,
      bindPacketsMs,
      totalWorkerMs:
        frame.requestStage.evaluatePrepareMs +
        frame.requestStage.requestInspectMs +
        lowerMs +
        bindPacketsMs,
    };
  } finally {
    const releaseStarted = performance.now();
    active.release_ticket(ticket);
    lastReleaseMs = performance.now() - releaseStarted;
  }
}

function postTaskError(id: string, request: ProductFrameRequest, error: unknown): void {
  scope.postMessage({
    protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
    type: "taskError",
    id,
    key: request.key,
    renderId: request.renderId,
    epoch: request.epoch,
    error: errorMessage(error),
  });
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? (error.stack ?? error.message) : String(error);
}

function bundleByteArrays(chunks: Uint8Array[]): {
  buffer: Uint8Array;
  views: Uint8Array[];
} {
  const bytes = chunks.reduce((total, chunk) => total + chunk.byteLength, 0);
  const buffer = new Uint8Array(bytes);
  const views: Uint8Array[] = [];
  let offset = 0;
  for (const chunk of chunks) {
    buffer.set(chunk, offset);
    views.push(buffer.subarray(offset, offset + chunk.byteLength));
    offset += chunk.byteLength;
  }
  return { buffer, views };
}
