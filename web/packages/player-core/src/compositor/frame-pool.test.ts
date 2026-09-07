import { expect, test } from "bun:test";

import {
  PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
  applyProductEngineBootstrap,
  assertProductFrameWorkerRequest,
  assertProductFrameWorkerResponse,
  emptyProductEngineBootstrap,
  parseProductRenderReceipt,
  productFrameWorkerFatalResponse,
  type ProductEngineWire,
  type ProductFrameWorkerRequest,
  type ProductFrameWorkerResponse,
} from "./frame-protocol.ts";
import { ProductFramePlannerPool, planningKey } from "./frame-pool.ts";

const RENDER_ID = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const STALE_RENDER_ID = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

class WrongRenderIdWorker {
  onmessage: ((event: MessageEvent<unknown>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;

  postMessage(message: ProductFrameWorkerRequest): void {
    if (message.type === "configure") {
      this.emit({
        protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
        type: "configured",
        id: message.id,
        renderId: message.bootstrap.expectedRenderId,
      });
      return;
    }
    const job = message.jobs[0]!;
    this.emit({
      protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
      type: "requests",
      id: job.id,
      stage: {
        key: job.request.key,
        renderId: STALE_RENDER_ID,
        epoch: job.request.epoch,
        frame: job.request.frame,
        generation: job.request.generation,
        requestPacket: new Uint8Array(),
        inspectionJson: "{}",
        evaluatePrepareMs: 0,
        requestInspectMs: 0,
        workerTurnGapMs: 0,
        previousReleaseMs: 0,
      },
    });
  }

  terminate(): void {}

  private emit(message: ProductFrameWorkerResponse): void {
    queueMicrotask(() => this.onmessage?.({ data: message } as MessageEvent<unknown>));
  }
}

class FatalBatchWorker {
  onmessage: ((event: MessageEvent<unknown>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  terminated = false;

  postMessage(message: ProductFrameWorkerRequest): void {
    if (message.type === "configure") {
      this.emit({
        protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
        type: "configured",
        id: message.id,
        renderId: message.bootstrap.expectedRenderId,
      });
      return;
    }
    this.emit(productFrameWorkerFatalResponse(
      message,
      "prepareBatch",
      new Error("bundle failed"),
    ));
  }

  terminate(): void {
    this.terminated = true;
  }

  private emit(message: ProductFrameWorkerResponse): void {
    queueMicrotask(() => this.onmessage?.({ data: message } as MessageEvent<unknown>));
  }
}

test("planning identity includes RenderId", () => {
  const input = {
    frame: 7,
    width: 1920,
    height: 1080,
    transparent: false,
    maxSurfaceBytes: 1024n,
    maxFrameBytes: 2048n,
  };
  expect(planningKey(RENDER_ID, 1, input)).not.toBe(planningKey(STALE_RENDER_ID, 1, input));
});

test("render receipt is narrow and requires positive safe result facts", () => {
  const receipt = {
    renderId: RENDER_ID,
    canvasWidth: 320,
    canvasHeight: 180,
    frameRate: "30/1",
    frameCount: 30,
    sampleRate: 48_000,
    sampleCount: 48_000,
  };
  expect(parseProductRenderReceipt(JSON.stringify(receipt))).toEqual(receipt);
  expect(parseProductRenderReceipt(JSON.stringify(receipt))).toMatchObject({
    canvasWidth: 320,
    canvasHeight: 180,
  });
  expect(() => parseProductRenderReceipt(JSON.stringify({ ...receipt, canvasWidth: 0 })))
    .toThrow("canvasWidth must be a positive safe integer");
  expect(() => parseProductRenderReceipt(JSON.stringify({
    ...receipt,
    canvasHeight: Number.MAX_SAFE_INTEGER + 1,
  }))).toThrow("canvasHeight must be a positive safe integer");
  expect(() => parseProductRenderReceipt(JSON.stringify({
    ...receipt,
    unsupportedIdentity: "sha256:unsupported",
  }))).toThrow("unknown field unsupportedIdentity");
});

test("worker protocol requires a strong RenderId at every boundary", () => {
  expect(() => assertProductFrameWorkerResponse({
    protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
    type: "configured",
    id: "configure-1",
  })).toThrow("must be a sha256 RenderId");
  expect(() => assertProductFrameWorkerRequest({
    protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
    type: "prepareBatch",
    id: "batch-1",
    jobs: [{ id: "frame-1", request: { renderId: "unsupported-bare-hex" } }],
  })).toThrow("must be a sha256 RenderId");
});

test("malformed and wrong-version requests map to an explicit protocol fatal response", () => {
  const malformed = productFrameWorkerFatalResponse(null, "protocol", new Error("request must be an object"));
  expect(malformed).toMatchObject({
    protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
    type: "fatal",
    id: "protocol",
    phase: "protocol",
  });
  expect(malformed.error).toContain("request must be an object");
  expect(() => assertProductFrameWorkerResponse(malformed)).not.toThrow();

  const wrongVersion = {
    protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION + 1,
    type: "prepareBatch",
    id: "batch-wrong-version",
    jobs: [],
  };
  expect(() => assertProductFrameWorkerRequest(wrongVersion)).toThrow("unsupported Product frame worker protocol");
  const fatal = productFrameWorkerFatalResponse(wrongVersion, "protocol", "unsupported protocol");
  expect(fatal).toMatchObject({
    type: "fatal",
    id: "batch-wrong-version",
    phase: "protocol",
    error: "unsupported protocol",
  });
  expect(() => assertProductFrameWorkerResponse(fatal)).not.toThrow();
});

test("bootstrap is versioned only by the enclosing worker protocol", () => {
  const bootstrap = emptyProductEngineBootstrap(
    "engine.js",
    "engine.wasm",
    "fixed-package",
    "timeline",
    "resource-manifest",
    "binding-bundle",
  );
  expect(bootstrap).toEqual({
    wasmModuleUrl: "engine.js",
    wasmUrl: "engine.wasm",
    fixedPackageManifestJson: "fixed-package",
    timelineJson: "timeline",
    resourceManifestJson: "resource-manifest",
    verifiedBindingBundleJson: "binding-bundle",
    expectedRenderId: "",
  });
  expect(PRODUCT_FRAME_WORKER_PROTOCOL_VERSION).toBe(1);
});

test("bootstrap opens exactly the four fixed-package members", () => {
  const calls: string[][] = [];
  const engine = {
    open_fixed_package: (...members: string[]) => {
      calls.push(members);
      return JSON.stringify({
        renderId: RENDER_ID,
        canvasWidth: 320,
        canvasHeight: 180,
        frameRate: "30/1",
        frameCount: 30,
        sampleRate: 48_000,
        sampleCount: 48_000,
      });
    },
  } as unknown as ProductEngineWire;
  const bootstrap = emptyProductEngineBootstrap(
    "engine.js",
    "engine.wasm",
    "fixed-package",
    "timeline",
    "resource-manifest",
    "binding-bundle",
  );
  bootstrap.expectedRenderId = RENDER_ID;

  expect(applyProductEngineBootstrap(engine, bootstrap)).toBe(RENDER_ID);
  expect(calls).toEqual([[
    "fixed-package",
    "timeline",
    "resource-manifest",
    "binding-bundle",
  ]]);
  expect(() => assertProductFrameWorkerRequest({
    protocolVersion: PRODUCT_FRAME_WORKER_PROTOCOL_VERSION,
    type: "configure",
    id: "configure-extra",
    bootstrap: { ...bootstrap, unexpected: "member" },
  })).toThrow("bootstrap contains unknown field unexpected");
});

test("planner rejects a worker stage from another RenderId", async () => {
  const worker = new WrongRenderIdWorker();
  let generation = 0n;
  const pool = new ProductFramePlannerPool({
    workerUrl: "frame-worker.js",
    size: 1,
    workerFactory: () => worker,
    allocateGeneration: () => {
      generation += 1n;
      return generation;
    },
  });
  const bootstrap = emptyProductEngineBootstrap(
    "engine.js",
    "engine.wasm",
    "{}",
    "{}",
    "{}",
    "{}",
  );
  bootstrap.expectedRenderId = RENDER_ID;
  await pool.init(bootstrap);

  const job = pool.prepare({
    frame: 0,
    width: 320,
    height: 180,
    transparent: false,
    maxSurfaceBytes: 1024n,
    maxFrameBytes: 2048n,
  });
  void job.ready.catch(() => undefined);
  await expect(job.requests).rejects.toThrow("request stage identity drift");
  pool.close();
});

test("planner rejects every pending promise after a batch-level worker failure", async () => {
  const worker = new FatalBatchWorker();
  let generation = 0n;
  const pool = new ProductFramePlannerPool({
    workerUrl: "frame-worker.js",
    size: 1,
    workerFactory: () => worker,
    allocateGeneration: () => {
      generation += 1n;
      return generation;
    },
  });
  const bootstrap = emptyProductEngineBootstrap(
    "engine.js",
    "engine.wasm",
    "{}",
    "{}",
    "{}",
    "{}",
  );
  bootstrap.expectedRenderId = RENDER_ID;
  await pool.init(bootstrap);

  const job = pool.prepare({
    frame: 0,
    width: 320,
    height: 180,
    transparent: false,
    maxSurfaceBytes: 1024n,
    maxFrameBytes: 2048n,
  });
  const readyError = job.ready.catch((error: unknown) => error);
  await expect(job.requests).rejects.toThrow("prepareBatch failure");
  const failure = await readyError;
  expect(failure).toBeInstanceOf(Error);
  expect((failure as Error).message).toContain("bundle failed");
  expect(worker.terminated).toBeTrue();
  expect(() => pool.prepare({
    frame: 1,
    width: 320,
    height: 180,
    transparent: false,
    maxSurfaceBytes: 1024n,
    maxFrameBytes: 2048n,
  })).toThrow("Product frame planner failed");
  pool.close();
});
