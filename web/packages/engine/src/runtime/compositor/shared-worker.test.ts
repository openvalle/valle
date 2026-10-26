import { expect, test } from "bun:test";
import { BrowserRuntimeBudget } from "../runtime-budget.ts";
import { sharedProductWorker } from "./shared-worker.ts";
import {
  PRODUCT_FRAME_WORKER_PROTOCOL_VERSION as version,
  type ProductEngineBootstrap,
  type ProductFrameWorkerRequest,
} from "./frame-protocol.ts";

const renderId = `sha256:${"1".repeat(64)}`;
const bootstrap: ProductEngineBootstrap = {
  wasmModuleUrl: "engine.js",
  wasmUrl: "engine.wasm",
  timelineJson: "timeline",
  fixedPackageManifestJson: "package",
  resourceManifestJson: "resources",
  verifiedBindingBundleJson: "bindings",
  expectedRenderId: renderId,
};
const snapshot = {
  receipt: {
    renderId,
    canvasWidth: 2,
    canvasHeight: 2,
    frameRate: "30/1",
    frameCount: 30,
    sampleRate: 48000,
    sampleCount: 48000,
  },
  audioProgramJson: "{}",
  resources: [],
};
const tick = () => Bun.sleep(0);

test("preview/export share one compile, keep independent ports, and prioritize pending interaction", async () => {
  const Original = globalThis.Worker;
  const physical: MockWorker[] = [];
  class MockWorker {
    onmessage: ((event: { data: unknown }) => void) | null = null;
    onerror: ((event: unknown) => void) | null = null;
    messages: ProductFrameWorkerRequest[] = [];
    terminated = false;
    constructor() {
      physical.push(this);
    }
    postMessage(message: ProductFrameWorkerRequest) {
      this.messages.push(message);
    }
    reply(message: unknown) {
      this.onmessage?.({ data: message });
    }
    terminate() {
      this.terminated = true;
    }
  }
  globalThis.Worker = MockWorker as unknown as typeof Worker;
  const budget = new BrowserRuntimeBudget({ cpuThreads: 1 });
  const output = sharedProductWorker("worker.js", bootstrap, budget, "export");
  const preview = sharedProductWorker(
    "worker.js",
    { ...bootstrap, gpuScene3d: true },
    budget,
    "interaction",
  );
  const replies: string[] = [];
  output.onmessage = (event) => replies.push(`export:${(event.data as any).id}`);
  preview.onmessage = (event) => replies.push(`preview:${(event.data as any).id}`);
  try {
    output.postMessage({ protocolVersion: version, type: "configure", id: "init", bootstrap });
    preview.postMessage({ protocolVersion: version, type: "configure", id: "init", bootstrap });
    await tick();
    expect(physical.length).toBe(1);
    expect(budget.snapshot().compiledRuntimes).toBe(1);
    const worker = physical[0]!;
    worker.reply({
      protocolVersion: version,
      type: "configured",
      id: worker.messages[0]!.id,
      renderId,
      snapshot,
    });
    await tick();
    expect(worker.messages.length).toBe(1);
    expect(replies).toEqual(["export:init", "preview:init"]);
    const call = (id: string): ProductFrameWorkerRequest => ({
      protocolVersion: version,
      type: "call",
      id,
      renderId,
      method: "audio_windows_json",
      args: [],
    });
    output.postMessage(call("first"));
    await tick();
    output.postMessage(call("queued-export"));
    preview.postMessage(call("seek"));
    worker.reply({
      protocolVersion: version,
      type: "callResult",
      id: worker.messages[1]!.id,
      renderId,
      result: "[]",
    });
    await tick();
    expect(worker.messages[2]!.id.endsWith(":seek")).toBe(true);
    output.terminate();
    expect(worker.terminated).toBe(false);
    worker.reply({
      protocolVersion: version,
      type: "callResult",
      id: worker.messages[2]!.id,
      renderId,
      result: "[]",
    });
    await tick();
    expect(worker.messages.length).toBe(3); // Retired client's queued work was removed.
    expect(replies.at(-1)).toBe("preview:seek");
    expect(budget.snapshot().peakCpuSlots).toBe(1);
    expect(budget.snapshot().activeCpuSlots).toBe(0);
    preview.terminate();
    expect(worker.terminated).toBe(true);
    expect(budget.snapshot().compiledRuntimes).toBe(0);
  } finally {
    output.terminate();
    preview.terminate();
    globalThis.Worker = Original;
  }
});

test("Motion capture releases the single CPU slot before kernels and resumes the original frame identity", async () => {
  const Original = globalThis.Worker,
    physical: MockWorker[] = [];
  class MockWorker {
    onmessage: ((event: { data: unknown }) => void) | null = null;
    onerror: ((event: unknown) => void) | null = null;
    messages: any[] = [];
    terminated = false;
    constructor() {
      physical.push(this);
    }
    postMessage(message: any) {
      this.messages.push(message);
    }
    reply(message: unknown) {
      this.onmessage?.({ data: message });
    }
    terminate() {
      this.terminated = true;
    }
  }
  globalThis.Worker = MockWorker as unknown as typeof Worker;
  const budget = new BrowserRuntimeBudget({ cpuThreads: 1, memoryBytes: 128 * 1024 ** 2 });
  const port = sharedProductWorker(
    "https://localhost/worker.js",
    { ...bootstrap, gpuScene3d: true },
    budget,
    "export",
  );
  const request = {
    realtime: false,
    deliveryOutput: true,
    key: "frame/key",
    renderId,
    epoch: 1,
    generation: 1n,
    frame: 7,
    width: 2,
    height: 2,
    transparent: false,
    maxSurfaceBytes: 4096n,
    maxFrameBytes: 4096n,
  };
  try {
    port.postMessage({
      protocolVersion: version,
      type: "prepareBatch",
      id: "batch",
      jobs: [{ id: "frame", request }],
    });
    await tick();
    const owner = physical[0]!,
      sent = owner.messages[0].jobs[0];
    expect(sent.request).toMatchObject({ realtime: false, deliveryOutput: true, gpuScene3d: true });
    owner.reply({
      protocolVersion: version,
      type: "motionWork",
      id: sent.id,
      request: sent.request,
      definitions: { [renderId]: { json: "scene", resourceBytes: new Uint8Array([1, 2, 3]) } },
      jobs: [
        { key: "clip", definitionId: renderId, inputJson: '{"viewport":{"width":2,"height":2}}' },
      ],
    });
    await tick();
    expect(physical.length).toBe(2);
    expect(budget.snapshot().activeCpuSlots).toBe(1);
    const kernel = physical[1]!,
      work = kernel.messages[0];
    expect(Object.hasOwn(work, "timelineJson")).toBe(false);
    kernel.reply({
      version,
      id: work.id,
      motion: new Uint8Array([1, 2]),
      workMs: 1,
      residentBytes: 1024,
    });
    await tick();
    const resumed = owner.messages[1].jobs[0];
    expect(resumed.id).toBe(sent.id);
    expect(resumed.request).toEqual(sent.request);
    expect(resumed.motionResults).toBeInstanceOf(Uint8Array);
    expect(budget.snapshot().peakCpuSlots).toBe(1);
    expect(budget.snapshot().compiledRuntimes).toBe(1);
    // A stale response cannot be substituted for the pending assembly.
    let errors = 0;
    port.onerror = () => errors++;
    owner.reply({
      protocolVersion: version,
      type: "motionWork",
      id: sent.id,
      request: { ...sent.request, epoch: 2 },
      definitions: {},
      jobs: [{ key: "clip", definitionId: renderId, inputJson: "{}" }],
    });
    expect(errors).toBe(1);
    expect(physical.every((worker) => worker.terminated)).toBe(true);
    expect(budget.snapshot().managedBytes).toBe(0);
    expect(budget.snapshot().activeCpuSlots).toBe(0);
  } finally {
    port.terminate();
    globalThis.Worker = Original;
  }
});

test("malformed shared runtime output fails every client and releases the host slot", async () => {
  const Original = globalThis.Worker;
  let worker: any;
  class BrokenWorker {
    onmessage: any;
    onerror: any;
    terminated = false;
    constructor() {
      // oxlint-disable-next-line typescript/no-this-alias -- The test drives the constructed worker's failure.
      worker = this;
    }
    postMessage() {}
    terminate() {
      this.terminated = true;
    }
  }
  globalThis.Worker = BrokenWorker as unknown as typeof Worker;
  const budget = new BrowserRuntimeBudget({ cpuThreads: 1 });
  const a = sharedProductWorker("broken.js", bootstrap, budget, "export"),
    b = sharedProductWorker("broken.js", bootstrap, budget, "interaction");
  let errors = 0;
  a.onerror = b.onerror = () => errors++;
  try {
    a.postMessage({ protocolVersion: version, type: "configure", id: "init", bootstrap });
    await tick();
    worker.onmessage({ data: { type: "callResult", id: "wrong-version" } });
    expect(errors).toBe(2);
    expect(worker.terminated).toBe(true);
    expect(budget.snapshot().activeCpuSlots).toBe(0);
  } finally {
    a.terminate();
    b.terminate();
    globalThis.Worker = Original;
  }
});
