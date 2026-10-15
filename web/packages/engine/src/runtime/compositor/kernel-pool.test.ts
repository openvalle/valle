import { expect, test } from "bun:test";
import { ProductKernelPool } from "./kernel-pool.ts";
import { BrowserRuntimeBudget } from "../runtime-budget.ts";
import { PRODUCT_FRAME_WORKER_PROTOCOL_VERSION as version, type ProductEngineBootstrap } from "./frame-protocol.ts";

const digest = `sha256:${"1".repeat(64)}`;
test("stateless kernels run under the shared CPU allowance and release memory after cancellation", async () => {
  const Original = globalThis.Worker, workers: MockWorker[] = [];
  class MockWorker {
    onmessage: ((e: any) => void) | null = null; onerror: ((e: any) => void) | null = null;
    message: any; terminated = false;
    constructor(readonly url: URL) { workers.push(this); }
    postMessage(message: any) { this.message = message; }
    terminate() { this.terminated = true; }
    reply() { this.onmessage?.({ data: { version, id: this.message.id, pixels: new Float32Array([1, 2]), workMs: 1, residentBytes: 1024 } }); }
  }
  globalThis.Worker = MockWorker as unknown as typeof Worker;
  const budget = new BrowserRuntimeBudget({ cpuThreads: 2, memoryBytes: 128 * 1024 ** 2 });
  const pool = new ProductKernelPool("https://localhost/worker.js", { wasmModuleUrl: "engine.js", wasmUrl: "engine.wasm" } as ProductEngineBootstrap, budget);
  const abort = new AbortController(), input = { content: digest, topology: digest, canonical: "{}", width: 2, height: 2 };
  try {
    const a = pool.run(input, "playback", abort.signal), b = pool.run(input, "playback", abort.signal);
    await Bun.sleep(0);
    expect(workers.length).toBe(2); expect(budget.snapshot().activeCpuSlots).toBe(2);
    expect(budget.snapshot().compiledRuntimes).toBe(0);
    expect(workers.every(w => w.url.searchParams.get("valle-kernel") === "1" && !Object.hasOwn(w.message, "timelineJson"))).toBe(true);
    workers[0]!.reply(); workers[1]!.reply();
    const results = await Promise.all([a, b]);
    expect(budget.snapshot().managedBytes).toBe(16); results.forEach(result => result.release());
    const cancelled = pool.run(input, "export", abort.signal).catch(e => e);
    await Bun.sleep(0); abort.abort(); expect((await cancelled).name).toBe("AbortError");
    // The posted coverage job drains with its permit; its worker and geometry cache survive.
    await Bun.sleep(0);
    expect(budget.snapshot().activeCpuSlots).toBe(1); expect(workers[0]!.terminated).toBe(false);
    workers[0]!.reply(); await Bun.sleep(0);
    expect(budget.snapshot().managedBytes).toBe(0); expect(budget.snapshot().activeCpuSlots).toBe(0);
    expect(workers.some(worker => worker.terminated)).toBe(false);
    expect(budget.snapshot().peakCpuSlots).toBe(2);
    expect(budget.snapshot().kernelJobs).toBe(3); expect(budget.snapshot().kernelWorkMs).toBe(3);
  } finally { pool.close(); globalThis.Worker = Original; }
  expect(budget.snapshot().kernelWorkers).toBe(0);
});

test("cancelled Motion work drains with its permit held, reuses definitions, and close retires a draining worker", async () => {
  const Original = globalThis.Worker, workers: MockWorker[] = [];
  class MockWorker {
    onmessage: ((e: any) => void) | null = null; onerror: ((e: any) => void) | null = null;
    message: any; terminated = false;
    constructor() { workers.push(this); }
    postMessage(message: any) { this.message = message; }
    terminate() { this.terminated = true; }
    reply() { this.onmessage?.({ data: { version, id: this.message.id, motion: new Uint8Array([1, 2, 3]), workMs: 2, residentBytes: 1024 } }); }
  }
  globalThis.Worker = MockWorker as unknown as typeof Worker;
  const budget = new BrowserRuntimeBudget({ cpuThreads: 1, memoryBytes: 128 * 1024 ** 2 });
  const pool = new ProductKernelPool("https://localhost/worker.js", { wasmModuleUrl: "engine.js", wasmUrl: "engine.wasm" } as ProductEngineBootstrap, budget);
  const input = { kind: "motion" as const, definitionId: digest, definitionJson: "immutable scene", inputJson: '{"viewport":{"width":2,"height":2}}', width: 2, height: 2, compact: false };
  try {
    const abort = new AbortController();
    const a = pool.runMotion(input, "playback", abort.signal).catch(error => error);
    await Bun.sleep(0); abort.abort();
    expect((await a).name).toBe("AbortError");
    expect(workers[0]!.terminated).toBe(false);
    expect(budget.snapshot().activeCpuSlots).toBe(1);
    const b = pool.runMotion(input, "interaction", new AbortController().signal);
    workers[0]!.reply(); await Bun.sleep(0);
    expect(workers.length).toBe(1);
    expect(workers[0]!.message.input.definitionJson).toBe("");
    workers[0]!.reply(); const result = await b;
    expect(budget.snapshot().managedBytes).toBe(6); result.release();
    const closeAbort = new AbortController();
    const c = pool.runMotion(input, "playback", closeAbort.signal).catch(error => error);
    await Bun.sleep(0); closeAbort.abort(); await c;
    pool.close(); await Bun.sleep(0);
    expect(workers[0]!.terminated).toBe(true);
    expect(budget.snapshot().activeCpuSlots).toBe(0);
    expect(budget.snapshot().managedBytes).toBe(0);
  } finally { pool.close(); globalThis.Worker = Original; }
});

test("work cancelled before reaching a worker keeps that idle worker and its caches", async () => {
  const Original = globalThis.Worker, workers: MockWorker[] = [];
  class MockWorker {
    onmessage: ((e: any) => void) | null = null; onerror: ((e: any) => void) | null = null;
    messages: any[] = []; terminated = false;
    constructor() { workers.push(this); }
    postMessage(message: any) { this.messages.push(message); }
    terminate() { this.terminated = true; }
    reply() { this.onmessage?.({ data: { version, id: this.messages.at(-1).id, pixels: new Float32Array([1]), workMs: 1, residentBytes: 1024 } }); }
  }
  globalThis.Worker = MockWorker as unknown as typeof Worker;
  const budget = new BrowserRuntimeBudget({ cpuThreads: 1, memoryBytes: 128 * 1024 ** 2 });
  const pool = new ProductKernelPool("https://localhost/worker.js", { wasmModuleUrl: "engine.js", wasmUrl: "engine.wasm" } as ProductEngineBootstrap, budget);
  const input = { content: digest, topology: digest, canonical: "{}", width: 2, height: 2 };
  try {
    const first = pool.run(input, "playback", new AbortController().signal);
    const abort = new AbortController();
    const skipped = pool.run(input, "playback", abort.signal).catch(error => error);
    await Bun.sleep(0); abort.abort();
    workers[0]!.reply(); (await first).release();
    expect((await skipped).name).toBe("AbortError");
    const next = pool.run(input, "playback", new AbortController().signal);
    await Bun.sleep(0);
    expect(workers.length).toBe(1); expect(workers[0]!.terminated).toBe(false);
    expect(workers[0]!.messages.length).toBe(2);
    workers[0]!.reply(); (await next).release();
    expect(budget.snapshot().managedBytes).toBe(0); expect(budget.snapshot().activeCpuSlots).toBe(0);
  } finally { pool.close(); globalThis.Worker = Original; }
});

test("an idle worker exits after the idle period and later work starts a fresh one", async () => {
  const Original = globalThis.Worker, workers: MockWorker[] = [];
  class MockWorker {
    onmessage: ((e: any) => void) | null = null; onerror: ((e: any) => void) | null = null;
    message: any; terminated = false;
    constructor() { workers.push(this); }
    postMessage(message: any) { this.message = message; }
    terminate() { this.terminated = true; }
    reply() { this.onmessage?.({ data: { version, id: this.message.id, motion: new Uint8Array([1]), workMs: 1, residentBytes: 4096 } }); }
  }
  globalThis.Worker = MockWorker as unknown as typeof Worker;
  const budget = new BrowserRuntimeBudget({ cpuThreads: 1, memoryBytes: 128 * 1024 ** 2 });
  const pool = new ProductKernelPool("https://localhost/worker.js", { wasmModuleUrl: "engine.js", wasmUrl: "engine.wasm" } as ProductEngineBootstrap, budget, 5);
  const input = { kind: "motion" as const, definitionId: digest, definitionJson: "immutable scene", inputJson: '{"viewport":{"width":2,"height":2}}', width: 2, height: 2, compact: false };
  try {
    const first = pool.runMotion(input, "playback", new AbortController().signal);
    await Bun.sleep(0); workers[0]!.reply(); (await first).release();
    expect(budget.snapshot().kernelResidentBytes).toBe(4096);
    // Work that arrives within the idle period reuses the worker and its definition.
    const second = pool.runMotion(input, "playback", new AbortController().signal);
    await Bun.sleep(0); expect(workers.length).toBe(1); expect(workers[0]!.message.input.definitionJson).toBe("");
    workers[0]!.reply(); (await second).release();
    await Bun.sleep(20);
    expect(workers[0]!.terminated).toBe(true);
    expect(budget.snapshot().kernelWorkers).toBe(0); expect(budget.snapshot().kernelResidentBytes).toBe(0);
    expect(budget.snapshot().peakKernelResidentBytes).toBe(4096);
    const third = pool.runMotion(input, "playback", new AbortController().signal);
    await Bun.sleep(0); expect(workers.length).toBe(2); expect(workers[1]!.message.input.definitionJson).toBe("immutable scene");
    workers[1]!.reply(); (await third).release();
  } finally { pool.close(); globalThis.Worker = Original; }
  expect(budget.snapshot().kernelWorkers).toBe(0);
});

test("worker heaps share the background half of the budget", async () => {
  const Original = globalThis.Worker, workers: MockWorker[] = [];
  const heap = 40 * 1024 ** 2;
  class MockWorker {
    onmessage: ((e: any) => void) | null = null; onerror: ((e: any) => void) | null = null;
    message: any; terminated = false;
    constructor() { workers.push(this); }
    postMessage(message: any) { this.message = message; }
    terminate() { this.terminated = true; }
    reply() { this.onmessage?.({ data: { version, id: this.message.id, pixels: new Float32Array([1]), workMs: 1, residentBytes: heap } }); }
  }
  globalThis.Worker = MockWorker as unknown as typeof Worker;
  // Half of 128 MiB holds one 40 MiB heap, not two.
  const budget = new BrowserRuntimeBudget({ cpuThreads: 4, memoryBytes: 128 * 1024 ** 2 });
  const pool = new ProductKernelPool("https://localhost/worker.js", { wasmModuleUrl: "engine.js", wasmUrl: "engine.wasm" } as ProductEngineBootstrap, budget);
  const input = { content: digest, topology: digest, canonical: "{}", width: 2, height: 2 };
  try {
    // Before any heap is known, concurrent work starts a worker per job.
    const a = pool.run(input, "playback", new AbortController().signal), b = pool.run(input, "playback", new AbortController().signal);
    await Bun.sleep(0); expect(workers.length).toBe(2);
    workers[0]!.reply(); (await a).release();
    expect(workers[0]!.terminated).toBe(false);
    // The second heap exceeds the share, so its worker exits once idle; the last one stays.
    workers[1]!.reply(); (await b).release();
    expect(workers[1]!.terminated).toBe(true);
    expect(budget.snapshot().kernelWorkers).toBe(1); expect(budget.snapshot().kernelResidentBytes).toBe(heap);
    // Known heaps keep concurrent work on the one worker.
    const c = pool.run(input, "playback", new AbortController().signal), d = pool.run(input, "playback", new AbortController().signal);
    await Bun.sleep(0); expect(workers.length).toBe(2);
    workers[0]!.reply(); (await c).release();
    await Bun.sleep(0); workers[0]!.reply(); (await d).release();
    expect(workers.length).toBe(2); expect(budget.snapshot().kernelJobs).toBe(4);
  } finally { pool.close(); globalThis.Worker = Original; }
});
