import { sharedProductWorker, type ProductWorkerPort } from "./shared-worker.ts";
import { BrowserRuntimeBudget, sharedBrowserRuntimeBudget, type RuntimePriority } from "../runtime-budget.ts";
import {
  PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
  PRODUCT_FRAME_TEMPLATE_CACHE_SIZE,
  assertProductFrameWorkerResponse,
  type ProductEngineBootstrap,
  type ProductRuntimeMethod,
  type ProductRuntimeSnapshot,
  type ProductFrameReadyStage,
  type ProductFrameRequest,
  type ProductFrameRequestStage,
  type ProductFrameWorkerRequest,
} from "./frame-protocol.ts";

const DEFAULT_READY_FRAMES = 64;
const DEFAULT_READY_BYTES = 96 * 1024 * 1024;
// A worker runs its dispatched batch synchronously. Keep that batch short so a seek or clock
// catch-up does not wait behind eight obsolete frames; the ready cache provides the lookahead.
// Keep a bounded 1.6-second cushion while the main thread consumes already-admitted frames.
// Planning and GPU execution use independent threads/resources; serial burst alternation leaves
// both idle for a material part of every playback second.
const READY_REFILL_THRESHOLD = 48;

interface WorkerLike extends ProductWorkerPort {
  onmessage: ((event: MessageEvent<unknown>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  postMessage(
    message: ProductFrameWorkerRequest,
    transfer?: Transferable[],
  ): void;
  terminate(): void;
}

interface WorkerSlot {
  index: number;
  worker: WorkerLike;
  tasks: Map<string, FrameTask>;
  templateHashes: Set<string>;
}

export interface ProductFramePlanningInput {
  trace?: boolean;
  frame: number;
  width: number;
  height: number;
  transparent: boolean;
  maxSurfaceBytes: bigint;
  maxFrameBytes: bigint;
}

export interface ProductFramePlanningJob {
  key: string;
  generation: bigint;
  requests: Promise<ProductFrameRequestStage>;
  ready: Promise<ProductFrameReadyStage>;
}

interface FrameTask extends ProductFramePlanningJob {
  request: ProductFrameRequest;
  priority: number;
  sequence: number;
  demanded: boolean;
  running: boolean;
  cancelled: boolean;
  requestsCompleted: boolean;
  readyCompleted: boolean;
  readyBytes: number;
  requestBytes: number;
  resolveRequests: (stage: ProductFrameRequestStage) => void;
  rejectRequests: (error: Error) => void;
  resolveReady: (stage: ProductFrameReadyStage) => void;
  rejectReady: (error: Error) => void;
}

interface ControlRequest {
  slot: WorkerSlot;
  resolve: (renderId: string) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

export interface ProductFramePlannerPoolOptions {
  workerUrl: string;
  runtimeBudget?: BrowserRuntimeBudget;
  priority?: RuntimePriority;
  maxReadyFrames?: number;
  maxReadyBytes?: number;
  workerFactory?: (url: string | URL, options: WorkerOptions) => WorkerLike;
  allocateGeneration: () => bigint;
}

export interface ProductFramePlannerStats {
  workerCount: number;
  readyHits: number;
  waitHits: number;
  queued: number;
  completed: number;
  cancelled: number;
  errors: number;
  queueDepth: number;
  readyFrames: number;
  readyBytes: number;
  workerMs: number;
}

export class ProductFramePlannerPool {
  private readonly workerUrl: string;
  private readonly maxReadyFrames: number;
  private readonly maxReadyBytes: number;
  private readonly workerFactory: ProductFramePlannerPoolOptions["workerFactory"];
  private readonly runtimeBudget: BrowserRuntimeBudget;
  private readonly priority: RuntimePriority;
  private readonly allocateGeneration: () => bigint;
  private readonly slots: WorkerSlot[] = [];
  private readonly tasks = new Map<string, FrameTask>();
  private readonly controls = new Map<string, ControlRequest>();
  private readonly calls = new Map<string, { resolve: (value: any) => void; reject: (error: Error) => void }>();
  initialization: ProductRuntimeSnapshot | null = null;
  private readonly templatePackets = new Map<string, Uint8Array>();
  private queue: FrameTask[] = [];
  private epoch = 1;
  private renderId: string | null = null;
  private sequence = 0;
  private messageId = 0;
  private readyBytes = 0;
  private estimatedFrameBytes = 0;
  private dispatchScheduled = false;
  private closed = false;
  private failure: Error | null = null;
  private counters = {
    readyHits: 0,
    waitHits: 0,
    queued: 0,
    completed: 0,
    cancelled: 0,
    errors: 0,
    workerMs: 0,
  };

  constructor({
    workerUrl,
    maxReadyFrames = DEFAULT_READY_FRAMES,
    maxReadyBytes = DEFAULT_READY_BYTES,
    workerFactory,
    runtimeBudget = sharedBrowserRuntimeBudget(),
    priority = "playback",
    allocateGeneration,
  }: ProductFramePlannerPoolOptions) {
    if (!workerUrl) throw new Error("Product frame worker URL is required");
    this.workerUrl = workerUrl;
    // One authoritative compiled runtime; CPU allowance controls shared host dispatch, not copies.
    this.maxReadyFrames = positiveInteger(maxReadyFrames, "maxReadyFrames");
    this.maxReadyBytes = positiveInteger(maxReadyBytes, "maxReadyBytes");
    this.workerFactory = workerFactory;
    this.runtimeBudget = runtimeBudget;
    this.priority = priority;
    this.allocateGeneration = allocateGeneration;
    this.runtimeBudget.registerPreparation(this, () => this.trimPreparedFrames());
  }

  async init(bootstrap: ProductEngineBootstrap): Promise<void> {
    this.assertUsable();
    if (this.slots.length > 0) return;
    for (let index = 0; index < 1; index += 1) {
      const worker = this.workerFactory
        ? this.workerFactory(this.workerUrl, { type: "module", name: `valle-product-frame-${index}` })
        : sharedProductWorker(this.workerUrl, bootstrap, this.runtimeBudget, this.priority);
      const slot: WorkerSlot = {
        index,
        worker,
        tasks: new Map(),
        templateHashes: new Set(),
      };
      worker.onmessage = (event) => this.onMessage(slot, event.data);
      worker.onerror = (event) =>
        this.fail(
          event.error instanceof Error
            ? event.error
            : new Error(
                event.message || `Product frame worker ${index} failed`,
              ),
        );
      this.slots.push(slot);
    }
    try {
      const renderIds = await Promise.all(
        this.slots.map((slot) => this.configure(slot, bootstrap)),
      );
      for (const openedRenderId of renderIds) {
        if (bootstrap.expectedRenderId && openedRenderId !== bootstrap.expectedRenderId) {
          throw new Error(
            `Product frame worker configured RenderId ${openedRenderId}, expected ${bootstrap.expectedRenderId}`,
          );
        }
      }
      this.renderId = renderIds[0]!;
    } catch (error) {
      this.fail(asError(error));
      throw error;
    }
  }

  call<T>(method: ProductRuntimeMethod, args: unknown[]): Promise<T> {
    this.assertUsable();
    if (this.calls.size >= 32) return Promise.reject(new Error("Product runtime call queue is full"));
    const slot = this.slots[0], renderId = this.renderId;
    if (!slot || !renderId) throw new Error("Product runtime is not configured");
    const id = `call-${++this.messageId}`;
    return new Promise<T>((resolve, reject) => {
      this.calls.set(id, { resolve, reject });
      try {
        const transfer: Transferable[] = [];
        const packed = copyRuntimeArguments(args, transfer) as unknown[];
        slot.worker.postMessage({ protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
          type: "call", id, renderId, method, args: packed }, transfer);
      }
      catch (error) { this.calls.delete(id); reject(asError(error)); }
    });
  }

  prefetch(
    input: ProductFramePlanningInput,
    priority = 1,
  ): ProductFramePlanningJob {
    const task = this.ensureTask(input, priority);
    task.requests.catch(() => undefined);
    task.ready.catch(() => undefined);
    return task;
  }

  prepare(
    input: ProductFramePlanningInput,
    priority = 0,
  ): ProductFramePlanningJob {
    const task = this.ensureTask(input, priority);
    if (!task.demanded) {
      task.demanded = true;
      if (task.readyCompleted) this.counters.readyHits += 1;
      else this.counters.waitHits += 1;
    }
    this.promote(task, priority);
    this.scheduleDispatch();
    return task;
  }

  retainPlaybackWindow(
    currentFrame: number,
    lastFrame: number,
    lookahead: number,
  ): void {
    const maximum = Math.min(
      lastFrame,
      currentFrame + Math.max(1, Math.trunc(lookahead)),
    );
    for (const task of [...this.tasks.values()]) {
      if (task.demanded) continue;
      if (task.request.frame >= currentFrame && task.request.frame <= maximum)
        continue;
      this.cancelTask(task, "Product frame prefetch left the playback window");
    }
  }

  trimPreparedFrames(): void { for (const task of [...this.tasks.values()]) if (!task.demanded) this.cancelTask(task, "Prepared frame evicted under memory pressure"); }

  adoptPreparationMemory(key: string): void { for (const slot of this.slots) slot.worker.releasePreparation?.(key); }

  release(key: string): void {
    const task = this.tasks.get(key);
    if (!task) return;
    this.removeTask(task);
    this.scheduleDispatch();
  }

  advanceEpoch(): void {
    this.epoch += 1;
    for (const task of [...this.tasks.values()]) {
      this.cancelTask(task, "Product frame planning epoch changed");
    }
  }

  snapshot(): ProductFramePlannerStats {
    let readyFrames = 0;
    for (const task of this.tasks.values())
      readyFrames += Number(task.readyCompleted);
    return {
      ...this.counters,
      workerCount: this.slots.length,
      queueDepth: this.queue.length,
      readyFrames,
      readyBytes: this.readyBytes,
    };
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.runtimeBudget.unregisterPreparation(this);
    for (const task of [...this.tasks.values()])
      this.cancelTask(task, "Product frame planner closed");
    for (const control of this.controls.values()) {
      clearTimeout(control.timer);
      control.reject(new Error("Product frame planner closed"));
    }
    this.controls.clear();
    for (const call of this.calls.values()) call.reject(new Error("Product runtime closed"));
    this.calls.clear();
    this.initialization = null;
    for (const slot of this.slots) slot.worker.terminate();
    this.slots.length = 0;
    this.renderId = null;
    this.queue = [];
    this.templatePackets.clear();
  }

  private ensureTask(
    input: ProductFramePlanningInput,
    priority: number,
  ): FrameTask {
    this.assertUsable();
    assertPlanningInput(input);
    const renderId = this.renderId;
    if (!renderId) throw new Error("Product frame planner is not configured");
    const key = planningKey(renderId, this.epoch, input);
    const existing = this.tasks.get(key);
    if (existing) {
      this.promote(existing, priority);
      return existing;
    }
    const generation = this.allocateGeneration();
    if (generation <= 0n)
      throw new Error("Product external generation id must be positive");
    let resolveRequests!: (stage: ProductFrameRequestStage) => void;
    let rejectRequests!: (error: Error) => void;
    let resolveReady!: (stage: ProductFrameReadyStage) => void;
    let rejectReady!: (error: Error) => void;
    const requests = new Promise<ProductFrameRequestStage>(
      (resolve, reject) => {
        resolveRequests = resolve;
        rejectRequests = reject;
      },
    );
    const ready = new Promise<ProductFrameReadyStage>((resolve, reject) => {
      resolveReady = resolve;
      rejectReady = reject;
    });
    const task: FrameTask = {
      key,
      generation,
      requests,
      ready,
      request: { key, renderId, epoch: this.epoch, generation, ...input },
      priority,
      sequence: this.sequence++,
      demanded: false,
      running: false,
      cancelled: false,
      requestsCompleted: false,
      readyCompleted: false,
      readyBytes: 0,
      requestBytes: 0,
      resolveRequests,
      rejectRequests,
      resolveReady,
      rejectReady,
    };
    this.tasks.set(key, task);
    this.queue.push(task);
    this.sortQueue();
    this.counters.queued += 1;
    this.scheduleDispatch();
    return task;
  }

  private scheduleDispatch(): void {
    if (this.dispatchScheduled || this.closed || this.failure) return;
    this.dispatchScheduled = true;
    queueMicrotask(() => {
      this.dispatchScheduled = false;
      this.dispatch();
    });
  }

  private dispatch(): void {
    if (this.closed || this.failure) return;
    for (const slot of this.slots) {
      // A frame can be waiting for a kernel or final assembly without consuming a CPU slot.
      // Keep enough bounded work across those stages; the shared budget limits actual compute.
      const inFlightLimit = Math.min(24, this.runtimeBudget.cpuThreads * 6);
      if (slot.tasks.size >= inFlightLimit) continue;
      const jobs: Array<{ id: string; request: ProductFrameRequest }> = [];
      while (slot.tasks.size < inFlightLimit) {
        const candidate = this.queue.find(
          (task) => !task.cancelled && this.tasks.get(task.key) === task,
        );
        const inFlight = this.slots.reduce(
          (n, worker) => n + worker.tasks.size,
          0,
        );
        const full =
          this.readyFrameCount() + inFlight >=
            Math.min(READY_REFILL_THRESHOLD, this.maxReadyFrames) ||
          this.readyBytes + (inFlight + 1) * this.estimatedFrameBytes >
            this.maxReadyBytes * 0.9;
        // A demanded frame must make progress even when speculative preparation is full.
        if (candidate && !candidate.demanded && full) break;
        let task: FrameTask | undefined;
        while ((task = this.queue.shift())) {
          if (!task.cancelled && this.tasks.get(task.key) === task) break;
        }
        if (!task) break;
        const id = `frame-${++this.messageId}`;
        task.running = true;
        slot.tasks.set(id, task);
        jobs.push({ id, request: task.request });
      }
      if (jobs.length === 0) continue;
      slot.worker.postMessage({
        protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
        type: "prepareBatch",
        id: `batch-${++this.messageId}`,
        jobs,
      });
    }
  }

  private configure(
    slot: WorkerSlot,
    bootstrap: ProductEngineBootstrap,
  ): Promise<string> {
    const id = `configure-${++this.messageId}`;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (!this.controls.delete(id)) return;
        reject(
          new Error(
            `Product frame worker ${slot.index} configuration timed out`,
          ),
        );
      }, 30_000);
      this.controls.set(id, { slot, resolve, reject, timer });
      slot.worker.postMessage({
        protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
        type: "configure",
        id,
        bootstrap,
      });
    });
  }

  private onMessage(slot: WorkerSlot, value: unknown): void {
    try {
      assertProductFrameWorkerResponse(value);
    } catch (error) {
      this.fail(asError(error));
      return;
    }
    const message = value;
    if (message.type === "motionWork") {
      this.fail(new Error("Motion kernel work must be routed through the shared runtime host"));
      return;
    }
    if (message.type === "callResult") {
      const call = this.calls.get(message.id);
      if (!call) return;
      this.calls.delete(message.id);
      if (message.renderId !== this.renderId) call.reject(new Error("Product runtime result RenderId drift"));
      else if (message.error) call.reject(new Error(message.error));
      else call.resolve(message.result);
      return;
    }
    if (message.type === "requestsBatch") {
      for (const entry of message.entries) {
        this.onMessage(slot, {
          protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
          type: "requests",
          id: entry.id,
          stage: entry.stage,
        });
      }
      return;
    }
    if (message.type === "readyBatch") {
      for (const entry of message.entries) {
        this.onMessage(slot, {
          protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
          type: "ready",
          id: entry.id,
          stage: entry.stage,
        });
      }
      return;
    }
    if (message.type === "configured") {
      const control = this.controls.get(message.id);
      if (!control || control.slot !== slot) return;
      this.controls.delete(message.id);
      clearTimeout(control.timer);
      if (message.error) control.reject(new Error(message.error));
      else if (!message.renderId) {
        control.reject(new Error("Product frame worker returned no RenderId"));
      } else {
        if (message.snapshot) this.initialization = message.snapshot;
        control.resolve(message.renderId);
      }
      return;
    }
    if (message.type === "fatal") {
      this.fail(
        new Error(
          `Product frame worker ${slot.index} ${message.phase} failure (${message.id}): ${message.error}`,
        ),
      );
      return;
    }

    const task = slot.tasks.get(message.id);
    if (!task) return;
    if (message.type === "requests") {
      if (
        message.stage.key !== task.key ||
        message.stage.renderId !== task.request.renderId ||
        message.stage.epoch !== task.request.epoch
      ) {
        this.fail(new Error("Product frame request stage identity drift"));
        return;
      }
      if (!task.cancelled) {
        task.requestsCompleted = true;
        task.requestBytes =
          message.stage.requestPacket.byteLength +
          Object.values(message.stage.gpuScene3d ?? {}).reduce(
            (sum, value) => sum + value.byteLength,
            0,
          );
        task.resolveRequests(message.stage);
      }
      return;
    }

    slot.tasks.delete(message.id);
    task.running = false;
    if (message.type === "taskError") {
      if (
        message.key !== task.key ||
        message.renderId !== task.request.renderId ||
        message.epoch !== task.request.epoch
      ) {
        this.fail(new Error("Product frame error stage identity drift"));
        return;
      }
      if (!task.cancelled) {
        this.counters.errors += 1;
        const error = new Error(message.error);
        if (!task.requestsCompleted) task.rejectRequests(error);
        task.rejectReady(error);
        this.removeTask(task);
      }
      this.scheduleDispatch();
      return;
    }
    if (
      message.stage.key !== task.key ||
      message.stage.renderId !== task.request.renderId ||
      message.stage.epoch !== task.request.epoch
    ) {
      this.fail(new Error("Product frame ready stage identity drift"));
      return;
    }
    // Mirror each worker's bounded LRU, including cancelled frames. Keep a packet while any
    // worker can still reference it without transferring it again.
    const transferredPlan = message.stage.planPacket;
    const templateKey = `${message.stage.renderId}:${message.stage.templateHash}`;
    if (transferredPlan) {
      const admittedPlan = transferredPlan.slice();
      this.templatePackets.set(templateKey, admittedPlan);
    }
    slot.templateHashes.delete(templateKey);
    slot.templateHashes.add(templateKey);
    if (slot.templateHashes.size > PRODUCT_FRAME_TEMPLATE_CACHE_SIZE) {
      const oldest = slot.templateHashes.values().next().value!;
      slot.templateHashes.delete(oldest);
      if (this.slots.every((worker) => !worker.templateHashes.has(oldest))) {
        this.templatePackets.delete(oldest);
      }
    }
    if (!task.cancelled) {
      if (!task.requestsCompleted) {
        this.fail(
          new Error(
            "Product frame became ready before its resource request stage",
          ),
        );
        return;
      }
      const planPacket = this.templatePackets.get(templateKey) ?? slot.worker.templatePacket?.(templateKey);
      if (!planPacket) {
        this.fail(
          new Error(
            `Product frame template '${message.stage.templateHash}' is absent`,
          ),
        );
        return;
      }
      const readyStage: ProductFrameReadyStage = {
        ...message.stage,
        planPacket,
        planPacketTransferred: transferredPlan !== null,
      };
      task.readyCompleted = true;
      task.readyBytes =
        (transferredPlan?.byteLength ?? 0) +
        task.requestBytes +
        readyStage.bindingPacket.byteLength +
        readyStage.schedulePacket.byteLength;
      this.readyBytes += task.readyBytes;
      this.estimatedFrameBytes = Math.max(
        task.readyBytes,
        this.estimatedFrameBytes * 0.9,
      );
      this.counters.completed += 1;
      this.counters.workerMs += readyStage.totalWorkerMs;
      task.resolveReady(readyStage);
      this.evictReadyFrames();
    }
    this.scheduleDispatch();
  }

  private evictReadyFrames(): void {
    while (
      this.readyFrameCount() > this.maxReadyFrames ||
      this.readyBytes > this.maxReadyBytes
    ) {
      const farthest = [...this.tasks.values()]
        .filter((task) => task.readyCompleted && !task.demanded)
        .sort((a, b) => b.priority - a.priority || b.sequence - a.sequence)[0];
      if (!farthest) return;
      this.cancelTask(farthest, "Product frame ready cache capacity exceeded");
    }
  }

  private readyFrameCount(): number {
    let count = 0;
    for (const task of this.tasks.values())
      count += Number(task.readyCompleted);
    return count;
  }

  private promote(task: FrameTask, priority: number): void {
    if (task.running || priority >= task.priority) return;
    task.priority = priority;
    this.sortQueue();
  }

  private sortQueue(): void {
    this.queue.sort(
      (a, b) => a.priority - b.priority || a.sequence - b.sequence,
    );
  }

  private cancelTask(task: FrameTask, reason: string): void {
    if (task.cancelled || this.tasks.get(task.key) !== task) return;
    task.cancelled = true;
    this.counters.cancelled += 1;
    const error = new Error(reason);
    if (!task.requestsCompleted) task.rejectRequests(error);
    if (!task.readyCompleted) task.rejectReady(error);
    this.removeTask(task);
  }

  private removeTask(task: FrameTask): void {
    this.adoptPreparationMemory(task.key);
    if (this.tasks.get(task.key) === task) this.tasks.delete(task.key);
    this.queue = this.queue.filter((queued) => queued !== task);
    if (task.readyBytes > 0) {
      this.readyBytes -= task.readyBytes;
      task.readyBytes = 0;
    }
  }

  private fail(error: Error): void {
    if (this.failure || this.closed) return;
    this.failure = error;
    for (const call of this.calls.values()) call.reject(error);
    this.calls.clear();
    this.counters.errors += 1;
    for (const control of this.controls.values()) {
      clearTimeout(control.timer);
      control.reject(error);
    }
    this.controls.clear();
    for (const task of [...this.tasks.values()]) {
      if (!task.requestsCompleted) task.rejectRequests(error);
      if (!task.readyCompleted) task.rejectReady(error);
      this.removeTask(task);
    }
    for (const slot of this.slots) slot.worker.terminate();
    this.slots.length = 0;
    this.renderId = null;
    this.queue = [];
    this.templatePackets.clear();
  }

  private assertUsable(): void {
    if (this.closed) throw new Error("Product frame planner is closed");
    if (this.failure)
      throw new Error(`Product frame planner failed: ${this.failure.message}`);
  }
}

/** A typed view can point into an entire WASM heap. Transfer a copy of only its visible bytes,
 * otherwise structured clone duplicates the whole heap for every small output stripe. */
export function copyRuntimeArguments(value: unknown, transfer: Transferable[]): unknown {
  if (ArrayBuffer.isView(value)) {
    const bytes = new Uint8Array(value.buffer, value.byteOffset, value.byteLength).slice();
    transfer.push(bytes.buffer);
    return new (value.constructor as { new(buffer: ArrayBuffer): ArrayBufferView })(bytes.buffer);
  }
  if (value instanceof ArrayBuffer) { const copy = value.slice(0); transfer.push(copy); return copy; }
  if (Array.isArray(value)) return value.map(entry => copyRuntimeArguments(entry, transfer));
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([key, entry]) => [key, copyRuntimeArguments(entry, transfer)]));
  return value;
}

export function planningKey(
  renderId: string,
  epoch: number,
  input: ProductFramePlanningInput,
): string {
  return [
    renderId,
    epoch,
    input.frame,
    input.width,
    input.height,
    Number(input.transparent),
    input.maxSurfaceBytes,
    input.maxFrameBytes,
  ].join(":");
}

function assertPlanningInput(input: ProductFramePlanningInput): void {
  if (!Number.isSafeInteger(input.frame) || input.frame < 0)
    throw new Error("frame must be a non-negative safe integer");
  positiveInteger(input.width, "width");
  positiveInteger(input.height, "height");
  if (
    input.maxSurfaceBytes <= 0n ||
    input.maxFrameBytes < input.maxSurfaceBytes
  ) {
    throw new Error("Product frame planning budgets are invalid");
  }
}

function positiveInteger(value: number, label: string): number {
  if (!Number.isSafeInteger(value) || value <= 0)
    throw new Error(`${label} must be a positive integer`);
  return value;
}

function asError(value: unknown): Error {
  return value instanceof Error ? value : new Error(String(value));
}
