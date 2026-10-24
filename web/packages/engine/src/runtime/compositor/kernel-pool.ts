import { BrowserRuntimeBudget, type RuntimePriority } from "../runtime-budget.ts";
import {
  PRODUCT_FRAME_WORKER_PROTOCOL_VERSION as version,
  type ProductEngineBootstrap,
} from "./frame-protocol.ts";
import type {
  KernelResult,
  Scene3dKernelInput,
  MotionKernelInput,
  KernelInput,
} from "./kernel-worker.ts";

type Lease = { release: () => void; shrink: (bytes: number) => void };
interface Work {
  id: number;
  input: KernelInput;
  priority: RuntimePriority;
  signal: AbortSignal;
  resolve: (value: { data: Float32Array | Uint8Array; release: () => void }) => void;
  reject: (error: unknown) => void;
}
interface Slot {
  worker: Worker;
  retire: () => void;
  resident: (bytes: number) => void;
  work?: Work;
  definitionId?: string;
  idle?: ReturnType<typeof setTimeout>;
}

/** A shared, lazy pool of independent kernels. Compiled timeline ownership stays in Host. A worker
 * that has no work for `idleMs` exits, returning its WASM heap; the budget bounds how many run. */
export class ProductKernelPool {
  private readonly slots: Slot[] = [];
  private queue: Work[] = [];
  private sequence = 0;
  private closed = false;
  private readonly abort = new AbortController();
  constructor(
    private readonly url: string,
    private readonly bootstrap: ProductEngineBootstrap,
    private readonly budget: BrowserRuntimeBudget,
    private readonly idleMs = 10_000,
  ) {}
  async run(
    input: Scene3dKernelInput,
    priority: RuntimePriority,
    signal: AbortSignal,
  ): Promise<{ pixels: Float32Array; release: () => void }> {
    const result = await this.submit(input, priority, signal);
    if (!(result.data instanceof Float32Array)) {
      result.release();
      throw new Error("invalid Scene3D result");
    }
    return { pixels: result.data, release: result.release };
  }
  async runMotion(
    input: MotionKernelInput,
    priority: RuntimePriority,
    signal: AbortSignal,
  ): Promise<{ bytes: Uint8Array; release: () => void }> {
    const result = await this.submit(input, priority, signal);
    if (!(result.data instanceof Uint8Array)) {
      result.release();
      throw new Error("invalid Motion result");
    }
    return { bytes: result.data, release: result.release };
  }
  private submit(
    input: KernelInput,
    priority: RuntimePriority,
    signal: AbortSignal,
  ): Promise<{ data: Float32Array | Uint8Array; release: () => void }> {
    signal.throwIfAborted();
    if (this.closed || this.queue.length >= 128)
      return Promise.reject(new Error("kernel dispatch unavailable"));
    return new Promise((resolve, reject) => {
      this.queue.push({ id: ++this.sequence, input, priority, signal, resolve, reject });
      this.dispatch();
    });
  }
  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.abort.abort();
    for (const job of this.queue) job.reject(new DOMException("Kernel host closed", "AbortError"));
    this.queue = [];
    for (const slot of this.slots) {
      clearTimeout(slot.idle);
      slot.worker.terminate();
      slot.retire();
    }
    this.slots.length = 0;
  }
  private dispatch(): void {
    if (this.closed) return;
    const order = { interaction: 0, playback: 1, export: 2, prefetch: 3 };
    this.queue.sort((a, b) => order[a.priority] - order[b.priority] || a.id - b.id);
    while (this.queue.length) {
      let slot = this.slots.find((slot) => !slot.work);
      if (
        !slot &&
        this.slots.length < Math.min(4, this.budget.cpuThreads) &&
        this.budget.admitsKernelWorker()
      ) {
        const url = new URL(this.url, globalThis.location?.href ?? import.meta.url);
        url.searchParams.set("valle-kernel", "1");
        const worker = new Worker(url, { type: "module", name: "valle-cpu-kernel" });
        slot = { worker, ...this.budget.retainKernelWorker(worker) };
        this.slots.push(slot);
      }
      if (!slot) return;
      clearTimeout(slot.idle);
      slot.idle = undefined;
      const job = this.queue.shift()!;
      slot.work = job;
      void this.execute(slot, job);
    }
  }
  private async execute(slot: Slot, job: Work): Promise<void> {
    const signal = AbortSignal.any([job.signal, this.abort.signal]);
    let memory: Lease | undefined, releaseCpu: (() => void) | undefined;
    // Once posted, the worker may be inside synchronous WASM for this job. Before that, an abort
    // leaves an idle worker whose definition, font, layout and geometry caches remain valid.
    let posted = false,
      healthy = true;
    const isMotion = "kind" in job.input && job.input.kind === "motion";
    try {
      // ROI coverage, output packet, vector growth and shadow scratch. The Rust kernel keeps
      // its original exact coverage algorithm; byte pressure reduces concurrency.
      const workingBytes = isMotion
        ? motionWorkingBytes(job.input as MotionKernelInput)
        : job.input.width * job.input.height * 128 + 8 * 1024 ** 2;
      memory = await this.budget.acquireKernel(workingBytes, signal);
      releaseCpu = await this.budget.acquireCpu(job.priority, signal);
      signal.throwIfAborted();
      const data = await new Promise<Float32Array | Uint8Array>((resolve, reject) => {
        const stop = () => {
          done();
          healthy = false;
          reject(this.abort.signal.reason);
        };
        // A seek retires the result immediately, but a running calculation drains with its
        // permit and bytes still held. Recreating definition, font, layout or prepared geometry
        // caches on every skipped frame costs more than finishing that bounded job. Closing the
        // host always terminates it; no permit is returned while synchronous WASM could still run.
        const abort = () => {
          job.reject(job.signal.reason);
        };
        const done = () => {
          job.signal.removeEventListener("abort", abort);
          this.abort.signal.removeEventListener("abort", stop);
          slot.worker.onmessage = null;
          slot.worker.onerror = null;
        };
        const fail = (error: Error) => {
          healthy = false;
          reject(error);
        };
        slot.worker.onmessage = (event) => {
          done();
          const reply = event.data as KernelResult;
          if (
            reply.version !== version ||
            reply.id !== job.id ||
            (!(isMotion
              ? reply.motion instanceof Uint8Array
              : reply.pixels instanceof Float32Array) &&
              !reply.error)
          )
            fail(new Error("invalid kernel response"));
          else if (reply.error) fail(new Error(reply.error));
          else {
            if (!Number.isFinite(reply.workMs) || reply.workMs! < 0) {
              fail(new Error("invalid kernel timing"));
              return;
            }
            if (!Number.isSafeInteger(reply.residentBytes) || reply.residentBytes! < 0) {
              fail(new Error("invalid kernel resident bytes"));
              return;
            }
            this.budget.recordKernelWork(reply.workMs!);
            slot.resident(reply.residentBytes!);
            resolve(isMotion ? reply.motion! : reply.pixels!);
          }
        };
        slot.worker.onerror = (event) => {
          done();
          fail(event.error ?? new Error(event.message));
        };
        job.signal.addEventListener("abort", abort, { once: true });
        this.abort.signal.addEventListener("abort", stop, { once: true });
        const input = isMotion
          ? {
              ...job.input,
              definitionJson:
                slot.definitionId === (job.input as MotionKernelInput).definitionId
                  ? ""
                  : (job.input as MotionKernelInput).definitionJson,
            }
          : job.input;
        if (isMotion) slot.definitionId = (job.input as MotionKernelInput).definitionId;
        posted = true;
        slot.worker.postMessage({
          version,
          id: job.id,
          wasmModuleUrl: this.bootstrap.wasmModuleUrl,
          wasmUrl: this.bootstrap.wasmUrl,
          input,
        });
      });
      signal.throwIfAborted();
      // Assembly owns a length-delimited copy alongside the returned packet until adoption.
      memory.shrink(data.byteLength * (isMotion ? 2 : 1));
      job.resolve({ data, release: memory.release });
      memory = undefined;
    } catch (error) {
      job.reject(error);
      // Replace only a failed worker, or one that may still be running this job when the host
      // closes. A later job then creates a fresh stateless kernel context.
      if (this.closed || (posted && !healthy)) {
        slot.worker.terminate();
        slot.retire();
        const index = this.slots.indexOf(slot);
        if (index >= 0) this.slots.splice(index, 1);
      }
    } finally {
      memory?.release();
      releaseCpu?.();
      slot.work = undefined;
      if (this.slots.includes(slot)) this.park(slot);
      this.dispatch();
    }
  }
  /** Beyond the budget's share an idle worker exits now, but never the last one. */
  private park(slot: Slot): void {
    if (this.slots.length > 1 && !this.budget.kernelWorkersFit()) {
      this.exit(slot);
      return;
    }
    slot.idle = setTimeout(() => this.exit(slot), this.idleMs);
  }
  private exit(slot: Slot): void {
    clearTimeout(slot.idle);
    slot.idle = undefined;
    const index = this.slots.indexOf(slot);
    if (index < 0 || slot.work) return;
    this.slots.splice(index, 1);
    slot.worker.terminate();
    slot.retire();
  }
}

function motionWorkingBytes(input: MotionKernelInput): number {
  const viewport = JSON.parse(input.inputJson).viewport;
  const width = Math.max(input.width, Number(viewport?.width));
  const height = Math.max(input.height, Number(viewport?.height));
  if (
    ![width, height].every(
      (value) => Number.isSafeInteger(value) && value > 0 && value <= 0x7fffffff,
    )
  )
    throw new Error("invalid Motion working extent");
  const bytes = width * height * 24 + 16 * 1024 ** 2 + input.definitionJson.length * 2;
  if (!Number.isSafeInteger(bytes)) throw new Error("Motion working set overflow");
  return bytes;
}
