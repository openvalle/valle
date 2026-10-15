import { expect, test } from "bun:test";
import { BrowserRuntimeBudget, sharedBrowserRuntimeBudget } from "./runtime-budget.ts";
import { copyRuntimeArguments } from "./compositor/frame-pool.ts";
import { executeRuntimeCall } from "./compositor/runtime-operations.ts";
import type { ProductEngineWire } from "./compositor/frame-protocol.ts";

test("one CPU slot gives pending interaction priority and cancellation never consumes a slot", async () => {
  const budget = new BrowserRuntimeBudget({ cpuThreads: 1, memoryBytes: 100 });
  const first = await budget.acquireCpu("export");
  const order: string[] = [];
  const background = budget.acquireCpu("export").then(release => { order.push("export"); release(); });
  const preview = budget.acquireCpu("interaction").then(release => { order.push("preview"); release(); });
  const abort = new AbortController();
  const cancelled = budget.acquireCpu("interaction", abort.signal).catch(error => error);
  abort.abort(); first(); first();
  expect((await cancelled).name).toBe("AbortError");
  await Promise.all([background, preview]);
  expect(order).toEqual(["preview", "export"]);
  expect(budget.snapshot().activeCpuSlots).toBe(0);
  expect(budget.snapshot().peakCpuSlots).toBe(1);
});

test("shared memory evicts completed storage before admitting the next frame and returns to zero", async () => {
  const budget = new BrowserRuntimeBudget({ cpuThreads: 4, memoryBytes: 100 });
  const a = {}, b = {};
  let trims = 0;
  budget.register(a, () => trims++); budget.register(b, () => trims++);
  budget.admitFrame(a, 80)(70);
  const finish = budget.admitFrame(b, 60);
  expect(trims).toBe(2);
  expect(budget.snapshot().managedBytes).toBe(60);
  expect(() => budget.admitFrame(a, 1)).toThrow("exclusive");
  finish(50); finish(50);
  expect(budget.snapshot().managedBytes).toBe(50);
  expect(() => budget.admitFrame(a, 101)).toThrow("exceeds");
  expect(() => budget.admitFrame({}, 1)).toThrow("registered");
  expect(budget.snapshot().peakManagedBytes).toBeLessThanOrEqual(100);
  budget.unregister(a); budget.unregister(b);
  expect(budget.snapshot().managedBytes).toBe(0);
});

test("a frame counts the storage it reuses once and borrows only what fits beside the rest", () => {
  const budget = new BrowserRuntimeBudget({ cpuThreads: 1, memoryBytes: 100 });
  const owner = {}, other = {};
  let trims = 0;
  budget.register(owner, () => trims++); budget.register(other, () => trims++);
  budget.admitFrame(owner, 60)(50);
  budget.admitFrame(other, 10)(20);
  // The owner's 50 retained bytes are surfaces this frame reuses: 70 + 20 fit, 70 + 50 + 20 do not.
  const finish = budget.admitFrame(owner, 70, 50);
  expect(trims).toBe(0);
  expect(budget.snapshot().managedBytes).toBe(90);
  expect(budget.borrowFrameBytes(10)).toBe(true);
  expect(budget.borrowFrameBytes(1)).toBe(false);
  expect(trims).toBe(0);
  expect(budget.snapshot().managedBytes).toBe(100);
  finish(60);
  expect(budget.snapshot().managedBytes).toBe(80);
  expect(() => budget.borrowFrameBytes(1)).toThrow("admitted frame");
  // Without the reuse the same frame drops completed storage first.
  budget.admitFrame(owner, 70)(0);
  expect(trims).toBe(2);
  expect(() => budget.admitFrame(owner, 10, 11)).toThrow("absorbed");
  expect(budget.snapshot().peakManagedBytes).toBeLessThanOrEqual(100);
  budget.unregister(owner); budget.unregister(other);
});

test("preview and export default to the same budget and different configurations stay explicit", () => {
  expect(sharedBrowserRuntimeBudget(2, 200)).toBe(sharedBrowserRuntimeBudget(2, 200));
  expect(sharedBrowserRuntimeBudget(1, 200)).not.toBe(sharedBrowserRuntimeBudget(2, 200));
});

test("kernel RPC transfers only a view's visible bytes and leaves caller memory usable", () => {
  const heap = new Float32Array(1024 * 1024);
  const stripe = heap.subarray(111, 115); stripe.set([1, 2, 3, 4]);
  const transfer: Transferable[] = [];
  const result = copyRuntimeArguments([{ pixels: stripe }], transfer) as Array<{ pixels: Float32Array }>;
  expect(result[0]!.pixels.buffer.byteLength).toBe(16);
  expect(transfer).toEqual([result[0]!.pixels.buffer]);
  const moved = structuredClone(result, { transfer });
  expect([...moved[0]!.pixels]).toEqual([1, 2, 3, 4]);
  expect([...stripe]).toEqual([1, 2, 3, 4]);
});

test("atomic worker audio windows release all PCM on success and failure", () => {
  const log: string[] = [];
  const engine = {
    clear_audio_pcm() { log.push("clear"); },
    register_audio_pcm_window(_id: string, _digest: string, start: bigint) { expect(start).toBe(96000n); log.push("register"); },
    mix_audio_pcm() { log.push("mix"); return new Float32Array([0.5, -0.5]); },
  };
  const args = ["render", 0n, 1n, [{ digest: "digest", startSample: 96000, left: new Float32Array(1), right: new Float32Array(1) }]];
  expect(executeRuntimeCall(engine as unknown as ProductEngineWire, "mix_audio_windows", args)).toEqual(new Float32Array([0.5, -0.5]));
  expect(log).toEqual(["clear", "register", "mix", "clear"]);
  engine.mix_audio_pcm = () => { throw new Error("mix failed"); };
  expect(() => executeRuntimeCall(engine as unknown as ProductEngineWire, "mix_audio_windows", args)).toThrow("mix failed");
  expect(log.at(-1)).toBe("clear");
});

test("async worker conversion returns its modified slice instead of losing the mutation", () => {
  const engine = { transform_srgb_preview_f16(pixels: Uint8Array) { pixels[0] = 7; } };
  const stripe = new Uint8Array([0, 1]);
  expect(executeRuntimeCall(engine as unknown as ProductEngineWire, "transform_srgb_preview_f16", [stripe, false])).toEqual(new Uint8Array([7, 1]));
});

test("background kernels share byte admission, shrink to packets, and yield to a large frame", async () => {
  const budget = new BrowserRuntimeBudget({ cpuThreads: 2, memoryBytes: 100 });
  const owner = {}; budget.register(owner, () => {});
  const first = await budget.acquireKernel(40);
  expect(budget.snapshot().managedBytes).toBe(40);
  let admitted = false;
  const waiting = budget.admitFrameAfterKernels(owner, 80).then(finish => { admitted = true; return finish; });
  await Bun.sleep(0); expect(admitted).toBe(false);
  first.shrink(10);
  const finish = await waiting;
  expect(budget.snapshot().managedBytes).toBe(90);
  const abort = new AbortController();
  const cancelled = budget.acquireKernel(20, abort.signal).catch(e => e);
  abort.abort(); expect((await cancelled).name).toBe("AbortError");
  first.release(); finish(0); budget.unregister(owner);
  expect(budget.snapshot().managedBytes).toBe(0);
  expect(budget.snapshot().peakManagedBytes).toBeLessThanOrEqual(100);
});

test("held kernel results never stall the next job: it borrows the idle host, prepared frames yield, then it fails", async () => {
  const budget = new BrowserRuntimeBudget({ cpuThreads: 2, memoryBytes: 100 });
  const prepared: Array<() => void> = [];
  budget.registerPreparation({}, () => { for (const release of prepared.splice(0)) release(); });
  // A prepared frame holds a finished job's result; with nothing running the next job may pass the half share.
  const first = await budget.acquireKernel(40); first.shrink(30); prepared.push(first.release);
  const second = await budget.acquireKernel(40);
  // While a job runs, the share holds.
  let third: Awaited<ReturnType<BrowserRuntimeBudget["acquireKernel"]>> | undefined;
  const waiting = budget.acquireKernel(20).then(lease => third = lease);
  await Bun.sleep(0); expect(third).toBeUndefined();
  second.shrink(20); prepared.push(second.release);
  await waiting; third!.shrink(10);
  // 60 held: a 60-byte job fits once the prepared frames give their results up.
  const fourth = await budget.acquireKernel(60);
  expect(budget.snapshot().kernelWorkingBytes).toBe(70);
  fourth.shrink(60);
  // Nothing in flight can make room for more.
  await expect(budget.acquireKernel(40)).rejects.toThrow("does not fit beside 70 bytes");
  // A frame in flight returns its bytes, so the job waits for it instead.
  fourth.release();
  const owner = {}; budget.register(owner, () => {});
  const finish = budget.admitFrame(owner, 50);
  let fifth = false;
  const late = budget.acquireKernel(45).then(lease => { fifth = true; return lease; });
  await Bun.sleep(0); expect(fifth).toBe(false);
  finish(0);
  (await late).release(); third!.release(); budget.unregister(owner);
  expect(budget.snapshot().managedBytes).toBe(0);
  expect(budget.snapshot().peakManagedBytes).toBeLessThanOrEqual(100);
});

test("3D render and on-demand picking retire full raster planes in the same worker job", () => {
  const log: string[] = [];
  const engine = {
    render_scene3d_request() { log.push("render"); return new Uint8Array([1, 2]); },
    scene3d_pick_json() { log.push("pick"); return '{"nodeId":1}'; },
    clear_scene3d_frames() { log.push("clear"); },
  };
  expect(executeRuntimeCall(engine as unknown as ProductEngineWire, "render_scene3d_request", [])).toEqual(new Uint8Array([1, 2]));
  expect(log).toEqual(["render", "clear"]); log.length = 0;
  expect(executeRuntimeCall(engine as unknown as ProductEngineWire, "pick_scene3d_request", ["content", "topology", new Uint8Array(), 0, 0])).toBe('{"nodeId":1}');
  expect(log).toEqual(["render", "pick", "clear"]);
  engine.scene3d_pick_json = () => { throw new Error("pick failed"); };
  expect(() => executeRuntimeCall(engine as unknown as ProductEngineWire, "pick_scene3d_request", [])).toThrow("pick failed");
  expect(log.at(-1)).toBe("clear");
});
