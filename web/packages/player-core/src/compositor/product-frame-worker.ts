import {
  PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
  applyProductEngineBootstrap,
  assertProductFrameWorkerRequest,
  productFrameWorkerFatalResponse,
  type ProductEngineWire,
  type ProductFrameRequest,
  type ProductFrameRequestStage,
  type ProductFrameWorkerReadyStage,
  type ProductFrameWorkerRequest,
  type ProductFrameWorkerResponse,
  type ProductWasmModule,
} from "./frame-protocol.ts";

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
const RESULT_BATCH_FRAMES = 8;

interface StartedFrame {
  id: string;
  request: ProductFrameRequest;
  ticket: number;
  requestStage: ProductFrameRequestStage;
}

scope.onmessage = (event) => {
  messageChain = messageChain.then(() => handleIncomingMessage(event.data)).catch((error) => {
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
    lastReadyAt = null;
    lastReleaseMs = 0;
    const wasm = await import(message.bootstrap.wasmModuleUrl) as ProductWasmModule;
    await wasm.default(message.bootstrap.wasmUrl);
    const canonicalTimelineJson = wasm.canonicalize_timeline_document(message.bootstrap.timelineJson);
    const bootstrap = { ...message.bootstrap, timelineJson: canonicalTimelineJson };
    const next = new wasm.ProductEngine();
    try {
      const openedRenderId = applyProductEngineBootstrap(next, bootstrap);
      engine = next;
      renderId = openedRenderId;
      scope.postMessage({
        protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
        type: "configured",
        id: message.id,
        renderId: openedRenderId,
      });
    } catch (error) {
      next.free();
      throw error;
    }
    return;
  }

  for (let offset = 0; offset < message.jobs.length; offset += RESULT_BATCH_FRAMES) {
    const jobs = message.jobs.slice(offset, offset + RESULT_BATCH_FRAMES);
    const started: StartedFrame[] = [];
    for (const [index, job] of jobs.entries()) {
      try {
        started.push(startFrame(job.id, job.request, index === 0));
      } catch (error) {
        postTaskError(job.id, job.request, error);
      }
    }
    if (started.length === 0) continue;
    const bundledRequests = bundleByteArrays(
      started.map((frame) => frame.requestStage.requestPacket),
    );
    scope.postMessage({
      protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
      type: "requestsBatch",
      id: `requests-${message.id}-${offset}`,
      entries: started.map((frame, index) => ({
        id: frame.id,
        stage: { ...frame.requestStage, requestPacket: bundledRequests.views[index]! },
      })),
    }, [bundledRequests.buffer.buffer]);

    const ready: Array<{ id: string; stage: ProductFrameWorkerReadyStage }> = [];
    for (const frame of started) {
      try {
        ready.push({ id: frame.id, stage: finishFrame(frame) });
      } catch (error) {
        postTaskError(frame.id, frame.request, error);
      }
    }
    if (ready.length === 0) continue;
    const chunks: Uint8Array[] = [];
    const packetIndexes: Array<{ plan: number | null; binding: number; schedule: number }> = [];
    for (const entry of ready) {
      const plan = entry.stage.planPacket === null ? null : chunks.push(entry.stage.planPacket) - 1;
      const binding = chunks.push(entry.stage.bindingPacket) - 1;
      const schedule = chunks.push(entry.stage.schedulePacket) - 1;
      packetIndexes.push({ plan, binding, schedule });
    }
    const bundledReady = bundleByteArrays(chunks);
    lastReadyAt = performance.now();
    scope.postMessage({
      protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
      type: "readyBatch",
      id: `ready-${message.id}-${offset}`,
      entries: ready.map((entry, index) => {
        const packets = packetIndexes[index]!;
        return {
          id: entry.id,
          stage: {
            ...entry.stage,
            planPacket: packets.plan === null ? null : bundledReady.views[packets.plan]!,
            bindingPacket: bundledReady.views[packets.binding]!,
            schedulePacket: bundledReady.views[packets.schedule]!,
          },
        };
      }),
    }, [bundledReady.buffer.buffer]);
  }
}

function startFrame(
  messageId: string,
  request: ProductFrameRequest,
  measureTurnGap: boolean,
): StartedFrame {
  const active = engine;
  const activeRenderId = renderId;
  if (!active || !activeRenderId) throw new Error("Product frame worker is not configured");
  if (request.renderId !== activeRenderId) {
    throw new Error(
      `Product frame request RenderId drift: expected ${activeRenderId}, got ${request.renderId}`,
    );
  }
  const workerTurnGapMs = !measureTurnGap || lastReadyAt === null
    ? 0
    : performance.now() - lastReadyAt;
  const evaluateStarted = performance.now();
  const ticket = active.evaluate_prepare_preview(
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
    if (transmitPlan) sentTemplateHashes.add(templateIdentity);
    const bindingPacket = active.binding_bytes(ticket);
    const schedulePacket = active.bound_schedule_bytes(ticket);
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
      lowerMs,
      bindPacketsMs,
      totalWorkerMs: frame.requestStage.evaluatePrepareMs
        + frame.requestStage.requestInspectMs
        + lowerMs
        + bindPacketsMs,
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
  return error instanceof Error ? error.stack ?? error.message : String(error);
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
