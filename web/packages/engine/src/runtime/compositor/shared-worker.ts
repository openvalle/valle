import { ProductKernelPool } from "./kernel-pool.ts";
import { BrowserRuntimeBudget, type RuntimePriority } from "../runtime-budget.ts";
import {
  PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
  assertProductFrameWorkerResponse,
  type ProductEngineBootstrap,
  type ProductFrameWorkerRequest,
  type ProductFrameWorkerResponse,
  type ProductRuntimeSnapshot,
  type MotionKernelDefinition,
} from "./frame-protocol.ts";

export interface ProductWorkerPort {
  onmessage: ((event: MessageEvent<unknown>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  postMessage(message: ProductFrameWorkerRequest, transfer?: Transferable[]): void;
  terminate(): void;
  templatePacket?(key: string): Uint8Array | undefined;
  releasePreparation?(key: string): void;
}
interface PreparedMemory {
  abort: AbortController;
  leases: Array<() => void>;
}
interface Job {
  port: Port;
  message: ProductFrameWorkerRequest;
  transfer: Transferable[];
  priority: RuntimePriority;
  frames: Map<string, PreparedMemory>;
  pending: Map<string, Promise<void>>;
}
const hosts = new WeakMap<BrowserRuntimeBudget, Map<string, Host>>();
let portSequence = 0;

/** Preview and export use independent seek/cache epochs but share one compiled WASM owner. */
export function sharedProductWorker(
  url: string,
  bootstrap: ProductEngineBootstrap,
  budget: BrowserRuntimeBudget,
  priority: RuntimePriority,
): ProductWorkerPort {
  let group = hosts.get(budget);
  if (!group) {
    group = new Map();
    hosts.set(budget, group);
  }
  const key = JSON.stringify([
    url,
    bootstrap.wasmModuleUrl,
    bootstrap.wasmUrl,
    bootstrap.fixedPackageManifestJson,
    bootstrap.timelineJson,
    bootstrap.resourceManifestJson,
    bootstrap.verifiedBindingBundleJson,
  ]);
  let host = group.get(key);
  if (!host) {
    host = new Host(url, bootstrap, budget, () => group!.delete(key));
    group.set(key, host);
  }
  return new Port(host, priority, bootstrap.gpuScene3d === true);
}

class Port implements ProductWorkerPort {
  readonly prefix = `runtime-${++portSequence}:`;
  onmessage: ProductWorkerPort["onmessage"] = null;
  onerror: ProductWorkerPort["onerror"] = null;
  closed = false;
  private readonly prepared = new Map<string, PreparedMemory>();
  constructor(
    readonly host: Host,
    readonly priority: RuntimePriority,
    readonly realtime: boolean,
  ) {
    host.ports.add(this);
  }
  postMessage(message: ProductFrameWorkerRequest, transfer: Transferable[] = []): void {
    if (this.closed) throw new Error("Product runtime port is closed");
    const wire: ProductFrameWorkerRequest =
      message.type === "prepareBatch"
        ? {
            ...message,
            id: this.prefix + message.id,
            jobs: message.jobs.map((job) => ({
              id: this.prefix + job.id,
              request: { ...job.request, realtime: this.realtime },
            })),
          }
        : { ...message, id: this.prefix + message.id };
    const frames = new Map<string, PreparedMemory>();
    if (wire.type === "prepareBatch")
      for (const job of wire.jobs) {
        const memory = { abort: new AbortController(), leases: [] as Array<() => void> };
        this.prepared.set(job.request.key, memory);
        frames.set(job.id, memory);
      }
    if (wire.type === "prepareBatch") {
      // A dispatch batch is a lookahead window, not an indivisible CPU job. Publish and release
      // each frame so foreground RPCs and other clients can run between preparations.
      for (const frame of wire.jobs)
        this.host.enqueue({
          port: this,
          message: { ...wire, jobs: [frame] },
          transfer: [],
          priority: this.priority,
          frames: new Map([[frame.id, frames.get(frame.id)!]]),
          pending: new Map(),
        });
    } else
      this.host.enqueue({
        port: this,
        message: wire,
        transfer,
        priority: this.priority,
        frames,
        pending: new Map(),
      });
  }
  deliver(message: ProductFrameWorkerResponse): void {
    if (this.closed) return;
    const strip = (id: string) => (id.startsWith(this.prefix) ? id.slice(this.prefix.length) : id);
    const local =
      message.type === "requestsBatch" || message.type === "readyBatch"
        ? {
            ...message,
            id: strip(message.id),
            entries: message.entries.map((entry) => ({ ...entry, id: strip(entry.id) })),
          }
        : { ...message, id: strip(message.id) };
    this.onmessage?.({ data: local } as MessageEvent<unknown>);
  }
  templatePacket(key: string) {
    return this.host.templates.get(key);
  }
  releasePreparation(key: string): void {
    const memory = this.prepared.get(key);
    if (!memory) return;
    this.prepared.delete(key);
    memory.abort.abort();
    for (const release of memory.leases) release();
    memory.leases = [];
  }
  terminate(): void {
    if (this.closed) return;
    this.closed = true;
    for (const key of this.prepared.keys()) this.releasePreparation(key);
    this.host.ports.delete(this);
    this.host.remove(this);
  }
}

class Host {
  readonly ports = new Set<Port>();
  readonly templates = new Map<string, Uint8Array>();
  private readonly worker: Worker;
  private queue: Job[] = [];
  private active: Job | null = null;
  private release: (() => void) | null = null;
  private readonly abort = new AbortController();
  private remaining = new Set<string>();
  private snapshot: ProductRuntimeSnapshot | null = null;
  private failed = false;
  private readonly releaseRuntime: () => void;
  private readonly kernels: ProductKernelPool;
  private readonly motionDefinitions = new Map<string, MotionKernelDefinition>();
  constructor(
    url: string,
    bootstrap: ProductEngineBootstrap,
    private readonly budget: BrowserRuntimeBudget,
    private readonly retire: () => void,
  ) {
    this.kernels = new ProductKernelPool(url, bootstrap, budget);
    this.worker = new Worker(url, { type: "module", name: "valle-product-runtime" });
    this.releaseRuntime = this.budget.retainRuntime(this);
    this.worker.onmessage = (event) => {
      try {
        assertProductFrameWorkerResponse(event.data);
        this.receive(event.data);
      } catch (error) {
        this.fail({ error, message: String(error) } as ErrorEvent);
      }
    };
    this.worker.onerror = (event) => this.fail(event);
  }
  private fail(event: ErrorEvent): void {
    try {
      for (const port of this.ports) port.onerror?.(event);
    } finally {
      this.stop();
    }
  }
  enqueue(job: Job): void {
    if (this.failed) throw new Error("Product runtime failed");
    if (this.queue.length >= 128) throw new Error("Product runtime dispatch queue is full");
    this.queue.push(job);
    void this.dispatch();
  }
  remove(port: Port): void {
    this.queue = this.queue.filter((job) => job.port !== port);
    if (!this.ports.size) this.stop();
  }
  private stop(): void {
    if (this.failed) return;
    this.failed = true;
    this.abort.abort();
    this.worker.terminate();
    this.kernels.close();
    this.worker.onmessage = null;
    this.worker.onerror = null;
    this.release?.();
    this.release = null;
    this.active = null;
    this.remaining.clear();
    this.queue = [];
    const ports = [...this.ports];
    for (const port of ports) port.terminate();
    this.ports.clear();
    this.templates.clear();
    this.motionDefinitions.clear();
    this.snapshot = null;
    this.releaseRuntime();
    this.retire();
  }
  private async dispatch(): Promise<void> {
    if (this.active || this.failed) return;
    const priority: Record<RuntimePriority, number> = {
      interaction: 0,
      playback: 1,
      export: 2,
      prefetch: 3,
    };
    // A frame already being presented may need a filter/resource RPC. Let that request finish
    // before spending the same client's allowance on future frames.
    this.queue.sort(
      (a, b) =>
        priority[a.priority] - priority[b.priority] ||
        Number(a.message.type === "prepareBatch") - Number(b.message.type === "prepareBatch") ||
        Number(a.message.type === "prepareBatch" && !a.message.jobs[0]?.motionResults) -
          Number(b.message.type === "prepareBatch" && !b.message.jobs[0]?.motionResults),
    );
    const job = this.queue.shift();
    if (!job) return;
    if (job.message.type === "prepareBatch") {
      // A small synchronous stage batch keeps the worker occupied while the foreground draws
      // the previous frame. Waiting for a main-thread acknowledgement after every 15 ms stage
      // otherwise serializes background work with CanvasKit's synchronous paint.
      const resumed = job.message.jobs[0]?.motionResults !== undefined;
      const limit = Math.min(4, this.budget.cpuThreads);
      while (job.message.jobs.length < limit) {
        const next = this.queue[0];
        if (
          !next ||
          next.port !== job.port ||
          next.message.type !== "prepareBatch" ||
          (next.message.jobs[0]?.motionResults !== undefined) !== resumed
        )
          break;
        this.queue.shift();
        job.message.jobs.push(...next.message.jobs);
        job.transfer.push(...next.transfer);
        for (const [id, memory] of next.frames) job.frames.set(id, memory);
      }
    }
    this.active = job;
    if (job.message.type === "prepareBatch") {
      job.message = {
        ...job.message,
        jobs: job.message.jobs.filter((frame) => {
          if (!job.frames.get(frame.id)?.abort.signal.aborted) return true;
          job.port.deliver({
            protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
            type: "taskError",
            id: frame.id,
            key: frame.request.key,
            epoch: frame.request.epoch,
            renderId: frame.request.renderId,
            error: "Frame preparation was cancelled",
          });
          return false;
        }),
      };
      if (!job.message.jobs.length) {
        this.complete();
        return;
      }
    }
    if (job.message.type === "configure" && this.snapshot) {
      const expected = job.message.bootstrap.expectedRenderId;
      job.port.deliver({
        protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
        type: "configured",
        id: job.message.id,
        ...(expected && expected !== this.snapshot.receipt.renderId
          ? { error: "Product shared runtime RenderId drift" }
          : { renderId: this.snapshot.receipt.renderId, snapshot: this.snapshot }),
      });
      this.complete();
      return;
    }
    try {
      this.release = await this.budget.acquireCpu(job.priority, this.abort.signal);
      if (this.failed || job.port.closed) {
        this.complete();
        return;
      }
      this.remaining = new Set(
        job.message.type === "prepareBatch"
          ? job.message.jobs.map((job) => job.id)
          : [job.message.id],
      );
      this.worker.postMessage(job.message, job.transfer);
    } catch (error) {
      if (!this.failed) job.port.onerror?.({ error, message: String(error) } as ErrorEvent);
      this.complete();
    }
  }
  private receive(message: ProductFrameWorkerResponse): void {
    const job = this.active;
    if (!job) return;
    if (message.type === "motionWork") {
      const frame =
        job.message.type === "prepareBatch"
          ? job.message.jobs.find((frame) => frame.id === message.id)
          : undefined;
      const memory = job.frames.get(message.id);
      if (
        !frame ||
        !memory ||
        message.request.key !== frame.request.key ||
        message.request.epoch !== frame.request.epoch ||
        message.request.renderId !== frame.request.renderId
      )
        throw new Error("Motion work identity drift");
      for (const [id, json] of Object.entries(message.definitions))
        this.motionDefinitions.set(id, json);
      const inputs = message.jobs.map((input) => {
        const definition = this.motionDefinitions.get(input.definitionId);
        if (!definition) throw new Error("Motion kernel definition missing");
        this.motionDefinitions.delete(input.definitionId);
        this.motionDefinitions.set(input.definitionId, definition);
        return {
          ...input,
          definitionJson: definition.json,
          resourceBytes: definition.resourceBytes,
        };
      });
      while (
        this.motionDefinitions.size > 4 ||
        [...this.motionDefinitions.values()].reduce(
          (sum, definition) => sum + definition.json.length + definition.resourceBytes.byteLength,
          0,
        ) >
          64 * 1024 ** 2
      ) {
        this.motionDefinitions.delete(this.motionDefinitions.keys().next().value!);
      }
      void (async () => {
        const results = await Promise.all(
          inputs.map(async (input) => {
            const result = await this.kernels.runMotion(
              {
                kind: "motion",
                definitionId: input.definitionId,
                definitionJson: input.definitionJson,
                resourceBytes: input.resourceBytes,
                inputJson: input.inputJson,
                compact: frame.request.realtime === true,
                width: frame.request.width,
                height: frame.request.height,
              },
              job.priority,
              memory.abort.signal,
            );
            if (memory.abort.signal.aborted) {
              result.release();
              memory.abort.signal.throwIfAborted();
            }
            memory.leases.push(result.release);
            return { key: input.key, bytes: result.bytes };
          }),
        );
        memory.abort.signal.throwIfAborted();
        const packet = packMotionResults(results);
        this.enqueue({
          ...job,
          message: {
            protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
            type: "prepareBatch",
            id: job.message.id,
            jobs: [{ ...frame, motionResults: packet }],
          },
          transfer: [packet.buffer as ArrayBuffer],
          pending: new Map(),
        });
      })().catch((error) => {
        if (this.failed) return;
        if (memory.abort.signal.aborted)
          job.port.deliver({
            protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
            type: "taskError",
            id: frame.id,
            key: frame.request.key,
            renderId: frame.request.renderId,
            epoch: frame.request.epoch,
            error: "Motion work was cancelled",
          });
        else this.fail({ error, message: String(error) } as ErrorEvent);
      });
      this.remaining.delete(message.id);
      if (!this.remaining.size) this.complete();
      return;
    }
    if (message.type === "configured" && message.snapshot) this.snapshot = message.snapshot;
    const ready =
      message.type === "readyBatch" ? message.entries : message.type === "ready" ? [message] : [];
    for (const entry of ready) {
      const key = `${entry.stage.renderId}:${entry.stage.templateHash}`;
      const packet = entry.stage.planPacket ?? this.templates.get(key);
      if (packet) {
        this.templates.delete(key);
        this.templates.set(key, packet);
      }
      while (this.templates.size > 64) this.templates.delete(this.templates.keys().next().value!);
    }
    if (message.type === "requestsBatch") {
      for (const entry of message.entries) {
        const memory = job.frames.get(entry.id);
        const projected = (async () => {
          if (memory?.abort.signal.aborted) return;
          const jobs = entry.stage.gpuScene3dJobs;
          if (jobs && memory) {
            const packets: Record<string, Float32Array> = {};
            await Promise.all(
              Object.entries(jobs).map(async ([key, input]) => {
                const result = await this.kernels.run(input, job.priority, memory.abort.signal);
                if (memory.abort.signal.aborted) {
                  result.release();
                  return;
                }
                memory.leases.push(result.release);
                packets[key] = result.pixels;
              }),
            );
            entry.stage.gpuScene3d = packets;
            delete entry.stage.gpuScene3dJobs;
          }
          if (!memory?.abort.signal.aborted) job.port.deliver({ ...message, entries: [entry] });
        })().catch((error) => {
          if (!memory?.abort.signal.aborted && !this.failed)
            this.fail({ error, message: String(error) } as ErrorEvent);
        });
        job.pending.set(entry.id, projected);
      }
    } else if (message.type === "readyBatch") {
      // Complete the authoritative CPU job immediately; coverage kernels keep independent
      // permits. Deliver each ready stage only after its resource packet, even after a seek.
      for (const entry of message.entries)
        void (job.pending.get(entry.id) ?? Promise.resolve()).then(() => {
          job.pending.delete(entry.id);
          if (!this.failed) job.port.deliver({ ...message, entries: [entry] });
        });
    } else job.port.deliver(message);
    if (message.type === "readyBatch")
      for (const entry of message.entries) this.remaining.delete(entry.id);
    else if (
      message.type === "ready" ||
      message.type === "taskError" ||
      message.type === "configured" ||
      message.type === "callResult"
    )
      this.remaining.delete(message.id);
    if (message.type === "fatal") {
      for (const port of this.ports) if (port !== job.port) port.deliver(message);
      this.stop();
      return;
    }
    if (!this.remaining.size) this.complete();
  }
  private complete(): void {
    this.release?.();
    this.release = null;
    this.active = null;
    this.remaining.clear();
    // Dispatch before CanvasKit's foreground continuation starts drawing. A timer here leaves
    // the background stages idle for the duration of the main thread's synchronous paint.
    void this.dispatch();
  }
}

/** Independent length-delimited records preserve arbitrary clip paths without JSON byte arrays. */
export function packMotionResults(results: Array<{ key: string; bytes: Uint8Array }>): Uint8Array {
  const fields = results.flatMap((result) => [new TextEncoder().encode(result.key), result.bytes]);
  const length = fields.reduce((sum, bytes) => sum + 4 + bytes.byteLength, 0);
  if (results.length > 128 || length > 128 * 1024 ** 2)
    throw new Error("Motion result packet exceeds budget");
  const packet = new Uint8Array(length),
    view = new DataView(packet.buffer);
  let offset = 0;
  for (const field of fields) {
    view.setUint32(offset, field.byteLength, true);
    offset += 4;
    packet.set(field, offset);
    offset += field.byteLength;
  }
  return packet;
}
