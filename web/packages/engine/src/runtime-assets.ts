export interface PlayerRuntimeAssets {
  engine: { glue: string; wasm: string };
  canvasKit: {
    full: { glue: string; wasm: string };
  };
  workers: { productFrame: string };
}

export type EngineRuntimeAssets = PlayerRuntimeAssets["engine"];

type RuntimeAssetsInput = {
  engine?: Partial<PlayerRuntimeAssets["engine"]>;
  canvasKit?: {
    full?: Partial<PlayerRuntimeAssets["canvasKit"]["full"]>;
  };
  workers?: Partial<PlayerRuntimeAssets["workers"]>;
};

type RuntimeAssetReader = (assets: RuntimeAssetsInput) => unknown;

const REQUIRED_URLS: ReadonlyArray<readonly [string, RuntimeAssetReader]> = [
  ["engine.glue", (assets) => assets.engine?.glue],
  ["engine.wasm", (assets) => assets.engine?.wasm],
  ["canvasKit.full.glue", (assets) => assets.canvasKit?.full?.glue],
  ["canvasKit.full.wasm", (assets) => assets.canvasKit?.full?.wasm],
  ["workers.productFrame", (assets) => assets.workers?.productFrame],
];

export function resolvePlayerRuntimeAssets(
  value: unknown,
  baseUrl: string | URL = import.meta.url,
): PlayerRuntimeAssets {
  if (!isRecord(value)) {
    throw new TypeError("runtimeAssets is required");
  }
  const input = value as RuntimeAssetsInput;
  for (const [name, read] of REQUIRED_URLS) {
    const url = read(input);
    if (typeof url !== "string" || url.length === 0) {
      throw new TypeError(`runtimeAssets.${name} must be a non-empty URL`);
    }
  }
  // Resolve same-origin host configuration against the page URL; use the module URL in environments
  // without a document.
  const documentUrl = typeof location === "undefined" ? import.meta.url : location.href;
  const resolvedBaseUrl = new URL(baseUrl, documentUrl);
  const assets = value as unknown as PlayerRuntimeAssets;
  const resolve = (url: string): string => new URL(url, resolvedBaseUrl).href;
  return {
    engine: {
      glue: resolve(assets.engine.glue),
      wasm: resolve(assets.engine.wasm),
    },
    canvasKit: {
      full: {
        glue: resolve(assets.canvasKit.full.glue),
        wasm: resolve(assets.canvasKit.full.wasm),
      },
    },
    workers: {
      productFrame: resolve(assets.workers.productFrame),
    },
  };
}

/** Compiler-only callers need the Engine module, but never load CanvasKit or a worker. */
export function resolveEngineRuntimeAssets(
  value: unknown,
  baseUrl: string | URL = import.meta.url,
): EngineRuntimeAssets {
  if (!isRecord(value)) throw new TypeError("runtimeAssets is required");
  const engine = value.engine;
  if (!isRecord(engine)) throw new TypeError("runtimeAssets.engine.glue must be a non-empty URL");
  for (const name of ["glue", "wasm"] as const) {
    if (typeof engine[name] !== "string" || engine[name].length === 0) {
      throw new TypeError(`runtimeAssets.engine.${name} must be a non-empty URL`);
    }
  }
  const documentUrl = typeof location === "undefined" ? import.meta.url : location.href;
  const resolvedBaseUrl = new URL(baseUrl, documentUrl);
  return {
    glue: new URL(engine.glue as string, resolvedBaseUrl).href,
    wasm: new URL(engine.wasm as string, resolvedBaseUrl).href,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
