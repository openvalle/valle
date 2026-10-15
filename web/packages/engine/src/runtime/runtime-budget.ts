/** Shared host admission. CPU slots count active worker messages, not physical CPU affinity.
 * Managed bytes cover admitted frame envelopes and retained host caches, not browser/driver RSS. */
export type RuntimePriority = "interaction" | "playback" | "export" | "prefetch";
const order: Record<RuntimePriority, number> = { interaction: 0, playback: 1, export: 2, prefetch: 3 };
type Release = () => void;

class Slots {
  active = 0;
  peak = 0;
  private sequence = 0;
  private queue: Array<{ priority: RuntimePriority; sequence: number; start: () => void; cancelled: boolean }> = [];
  constructor(private readonly limit: number) {}
  acquire(priority: RuntimePriority, signal?: AbortSignal): Promise<Release> {
    signal?.throwIfAborted();
    return new Promise((resolve, reject) => {
      const job = { priority, sequence: this.sequence++, cancelled: false, start: () => {
        signal?.removeEventListener("abort", abort);
        this.active++; this.peak = Math.max(this.peak, this.active);
        let released = false;
        resolve(() => { if (released) return; released = true; this.active--; this.drain(); });
      } };
      const abort = () => { job.cancelled = true; reject(signal?.reason ?? new DOMException("Cancelled", "AbortError")); this.drain(); };
      signal?.addEventListener("abort", abort, { once: true });
      this.queue.push(job); this.drain();
    });
  }
  private drain(): void {
    this.queue = this.queue.filter(job => !job.cancelled);
    this.queue.sort((a, b) => order[a.priority] - order[b.priority] || a.sequence - b.sequence);
    while (this.active < this.limit && this.queue.length) this.queue.shift()!.start();
  }
}

export class BrowserRuntimeBudget {
  readonly cpuThreads: number;
  readonly memoryBytes: number;
  private readonly cpu: Slots;
  private readonly presentation = new Slots(1);
  private readonly retained = new Map<object, { bytes: number; trim: () => void }>();
  private readonly runtimes = new Set<object>();
  private working = 0;
  private kernels = 0;
  /** Lease bytes of kernels still running; a lease shrunk to its result no longer counts. */
  private runningKernels = 0;
  private kernelJobs = 0;
  private kernelWorkMs = 0;
  recordKernelWork(milliseconds: number): void {
    if (!Number.isFinite(milliseconds) || milliseconds < 0) throw new RangeError("invalid kernel timing");
    this.kernelJobs++; this.kernelWorkMs += milliseconds;
  }
  private pendingFrame = 0;
  private readonly memoryWaiters = new Set<() => void>();
  /** WASM heap bytes per kernel worker, as last reported. */
  private readonly kernelWorkers = new Map<object, number>();
  private kernelResidentPeak = 0;
  private readonly preparationEvictions = new Map<object, () => void>();
  registerPreparation(owner: object, evict: () => void): void { this.preparationEvictions.set(owner, evict); }
  unregisterPreparation(owner: object): void { this.preparationEvictions.delete(owner); }
  private peak = 0;
  constructor(options: { cpuThreads?: number; memoryBytes?: number } = {}) {
    this.cpuThreads = options.cpuThreads ?? Math.max(1, Math.min(8, (globalThis.navigator?.hardwareConcurrency ?? 2) - 1));
    this.memoryBytes = options.memoryBytes ?? 2 * 1024 ** 3;
    if (!Number.isSafeInteger(this.cpuThreads) || this.cpuThreads < 1 || this.cpuThreads > 8) throw new RangeError("cpuThreads must be in 1..8");
    if (!Number.isSafeInteger(this.memoryBytes) || this.memoryBytes <= 0) throw new RangeError("memoryBytes must be positive");
    this.cpu = new Slots(this.cpuThreads);
  }
  acquireCpu(priority: RuntimePriority, signal?: AbortSignal): Promise<Release> { return this.cpu.acquire(priority, signal); }
  acquirePresentation(priority: RuntimePriority, signal?: AbortSignal): Promise<Release> { return this.presentation.acquire(priority, signal); }
  /** Worker ownership diagnostics; one authoritative compiled runtime can serve many players. */
  retainRuntime(owner: object): Release { this.runtimes.add(owner); return () => { this.runtimes.delete(owner); }; }
  retainKernelWorker(owner: object): { retire: Release; resident: (bytes: number) => void } {
    this.kernelWorkers.set(owner, 0);
    return {
      retire: () => { this.kernelWorkers.delete(owner); },
      resident: (bytes) => {
        if (!Number.isSafeInteger(bytes) || bytes < 0) throw new RangeError("invalid kernel resident bytes");
        if (!this.kernelWorkers.has(owner)) return;
        this.kernelWorkers.set(owner, bytes); this.kernelResidentPeak = Math.max(this.kernelResidentPeak, this.kernelResidentBytes());
      },
    };
  }
  /** Each kernel worker prepares its own copy of the scene, and its WASM heap keeps its high-water
   * mark until the worker exits. The heaps share the background half of the budget: another worker
   * starts only while one more heap as large as the largest still fits, and idle workers beyond
   * the share exit. The first worker always starts. */
  admitsKernelWorker(): boolean {
    let total = 0, largest = 0;
    for (const bytes of this.kernelWorkers.values()) { total += bytes; largest = Math.max(largest, bytes); }
    return this.kernelWorkers.size === 0 || total + largest <= this.memoryBytes / 2;
  }
  kernelWorkersFit(): boolean { return this.kernelResidentBytes() <= this.memoryBytes / 2; }
  private kernelResidentBytes(): number { let n = 0; for (const bytes of this.kernelWorkers.values()) n += bytes; return n; }
  private changed(): void { for (const wake of [...this.memoryWaiters]) wake(); }
  private waitMemory(signal?: AbortSignal): Promise<void> {
    signal?.throwIfAborted();
    return new Promise((resolve, reject) => {
      const done = () => { this.memoryWaiters.delete(wake); signal?.removeEventListener("abort", abort); };
      const wake = () => { done(); resolve(); };
      const abort = () => { done(); reject(signal?.reason ?? new DOMException("Cancelled", "AbortError")); };
      this.memoryWaiters.add(wake); signal?.addEventListener("abort", abort, { once: true });
    });
  }
  /** Independent CPU kernels hold no compiled timeline. Waiting for bytes holds no CPU slot.
   * A lease runs until it shrinks to its result, which a prepared frame holds until adoption. */
  async acquireKernel(bytes: number, signal?: AbortSignal): Promise<{ release: Release; shrink: (bytes: number) => void }> {
    if (!Number.isSafeInteger(bytes) || bytes <= 0 || bytes > this.memoryBytes) throw new RangeError("kernel working set exceeds managed memory budget");
    while (true) {
      signal?.throwIfAborted();
      // Keep a bounded background share. With no kernel running, held results return only once
      // their frames complete, so the next job may borrow the idle host.
      const quota = Math.max(Math.floor(this.memoryBytes / 2), bytes);
      if (this.working === 0 && bytes + this.kernels + this.retainedBytes() > this.memoryBytes) {
        for (const cached of this.retained.values()) { cached.trim(); cached.bytes = 0; }
      }
      const fits = this.kernels + bytes + Math.max(this.working, this.pendingFrame) + this.retainedBytes() <= this.memoryBytes;
      if (fits && (this.kernels + bytes <= quota || this.runningKernels === 0)) {
        this.kernels += bytes; this.runningKernels += bytes; this.peak = Math.max(this.peak, this.kernels + this.working + this.retainedBytes());
        let released = false, running = true;
        const settle = () => { if (running) { running = false; this.runningKernels -= bytes; } };
        return { release: () => { if (!released) { released = true; settle(); this.kernels -= bytes; this.changed(); } },
          shrink: (remaining) => { if (released || !Number.isSafeInteger(remaining) || remaining < 0 || remaining > bytes) throw new RangeError("invalid kernel lease shrink"); settle(); this.kernels -= bytes - remaining; bytes = remaining; this.changed(); } };
      }
      if (!fits && this.runningKernels === 0 && this.working === 0 && this.pendingFrame === 0) {
        // Nothing in flight will return bytes. Prepared frames give up their results; those of
        // the frame being presented wait for jobs like this one, so the lease fails instead.
        const held = this.kernels;
        for (const evict of this.preparationEvictions.values()) evict();
        signal?.throwIfAborted();
        if (this.kernels < held) continue;
        throw new RangeError(`kernel working set ${bytes} does not fit beside ${held} bytes held for frames in preparation, budget ${this.memoryBytes}`);
      }
      await this.waitMemory(signal);
    }
  }
  async admitFrameAfterKernels(owner: object, bytes: number, signal?: AbortSignal, absorbed = 0): Promise<(retainedBytes: number) => void> {
    if (!Number.isSafeInteger(bytes) || bytes <= 0 || bytes > this.memoryBytes) return this.admitFrame(owner, bytes, absorbed);
    this.pendingFrame = Number.isSafeInteger(absorbed) && absorbed >= 0 ? Math.max(0, bytes - absorbed) : bytes;
    try {
      if (this.kernels + bytes > this.memoryBytes) for (const evict of this.preparationEvictions.values()) evict();
      while (this.kernels + bytes > this.memoryBytes) await this.waitMemory(signal);
      signal?.throwIfAborted();
      return this.admitFrame(owner, bytes, absorbed);
    } finally { this.pendingFrame = 0; this.changed(); }
  }
  register(owner: object, trim: () => void): void {
    if (this.retained.has(owner)) throw new Error("runtime memory owner already registered");
    this.retained.set(owner, { bytes: 0, trim });
  }
  unregister(owner: object): void { this.retained.delete(owner); this.changed(); }
  private retainedBytes(): number { let n = 0; for (const owner of this.retained.values()) n += owner.bytes; return n; }
  /**
   * Called with presentation ownership, before resource fulfillment or working-surface allocation.
   * `absorbed` bytes of the owner's retained storage, such as pooled surfaces the frame reuses,
   * count inside `bytes` while the frame runs instead of beside it.
   */
  admitFrame(owner: object, bytes: number, absorbed = 0): (retainedBytes: number) => void {
    const own = this.retained.get(owner);
    if (!own) throw new Error("runtime memory owner is not registered");
    if (!Number.isSafeInteger(bytes) || bytes <= 0 || bytes > this.memoryBytes) {
      throw new RangeError(`frame working set ${bytes} exceeds managed memory budget ${this.memoryBytes}`);
    }
    if (!Number.isSafeInteger(absorbed) || absorbed < 0 || absorbed > bytes) throw new RangeError("invalid absorbed frame bytes");
    if (this.working !== 0) throw new Error("frame byte admission requires exclusive presentation ownership");
    if (bytes + this.kernels > this.memoryBytes) throw new RangeError("frame must wait for background kernels");
    if (bytes + this.kernels + this.retainedBytes() - Math.min(absorbed, own.bytes) > this.memoryBytes) {
      // Completed-frame storage is optional. Drop it before denying a large frame, preserving
      // the authoritative worker, compiled program and physical graphics contexts.
      for (const cached of this.retained.values()) { cached.trim(); cached.bytes = 0; }
    }
    own.bytes -= Math.min(absorbed, own.bytes);
    this.working = bytes;
    this.peak = Math.max(this.peak, bytes + this.kernels + this.retainedBytes());
    let released = false;
    return (retainedBytes) => {
      if (released) return; released = true;
      this.working = 0;
      const cached = this.retained.get(owner);
      if (cached) {
        if (!Number.isSafeInteger(retainedBytes) || retainedBytes < 0) {
          cached.trim(); cached.bytes = 0; throw new RangeError("invalid retained memory byte count");
        }
        cached.bytes = retainedBytes;
        if (this.retainedBytes() + this.kernels > this.memoryBytes) { cached.trim(); cached.bytes = 0; }
      }
      this.peak = Math.max(this.peak, this.kernels + this.retainedBytes()); this.changed();
    };
  }
  /**
   * Bytes an admitted frame takes beyond its admission, such as working surfaces it keeps for
   * reuse, granted only if they fit beside everything else. Nothing is trimmed: the frame may be
   * using the owner's retained storage.
   */
  borrowFrameBytes(bytes: number): boolean {
    if (!Number.isSafeInteger(bytes) || bytes <= 0) throw new RangeError("invalid frame borrow");
    if (this.working === 0) throw new Error("only an admitted frame may borrow memory");
    if (this.working + bytes + this.kernels + this.retainedBytes() > this.memoryBytes) return false;
    this.working += bytes;
    this.peak = Math.max(this.peak, this.working + this.kernels + this.retainedBytes());
    return true;
  }
  snapshot() { return { cpuThreads: this.cpuThreads, activeCpuSlots: this.cpu.active, peakCpuSlots: this.cpu.peak,
    memoryBytes: this.memoryBytes, managedBytes: this.working + this.kernels + this.retainedBytes(), peakManagedBytes: this.peak,
    memoryOwners: this.retained.size, preparationOwners: this.preparationEvictions.size, compiledRuntimes: this.runtimes.size, kernelWorkers: this.kernelWorkers.size, kernelResidentBytes: this.kernelResidentBytes(), peakKernelResidentBytes: this.kernelResidentPeak, kernelWorkingBytes: this.kernels, kernelJobs: this.kernelJobs, kernelWorkMs: this.kernelWorkMs }; }
}

const defaults = new Map<string, WeakRef<BrowserRuntimeBudget>>();
export function sharedBrowserRuntimeBudget(cpuThreads?: number, memoryBytes?: number): BrowserRuntimeBudget {
  const key = `${cpuThreads ?? "default"}:${memoryBytes ?? "default"}`;
  let budget = defaults.get(key)?.deref();
  if (!budget) { budget = new BrowserRuntimeBudget({ cpuThreads, memoryBytes }); defaults.set(key, new WeakRef(budget)); }
  return budget;
}
