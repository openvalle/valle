export interface PlayerRuntimeAssets {
  engine: { glue: string; wasm: string };
  canvasKit: {
    base: { glue: string; wasm: string };
    full: { glue: string; wasm: string };
  };
  fonts: { defaultSans: string };
  workers: { productFrame: string };
}

type RuntimeAssetsInput = {
  engine?: Partial<PlayerRuntimeAssets["engine"]>;
  canvasKit?: {
    base?: Partial<PlayerRuntimeAssets["canvasKit"]["base"]>;
    full?: Partial<PlayerRuntimeAssets["canvasKit"]["full"]>;
  };
  fonts?: Partial<PlayerRuntimeAssets["fonts"]>;
  workers?: Partial<PlayerRuntimeAssets["workers"]>;
};

type RuntimeAssetReader = (assets: RuntimeAssetsInput) => unknown;

const REQUIRED_URLS: ReadonlyArray<readonly [string, RuntimeAssetReader]> = [
  ["engine.glue", (assets) => assets.engine?.glue],
  ["engine.wasm", (assets) => assets.engine?.wasm],
  ["canvasKit.base.glue", (assets) => assets.canvasKit?.base?.glue],
  ["canvasKit.base.wasm", (assets) => assets.canvasKit?.base?.wasm],
  ["canvasKit.full.glue", (assets) => assets.canvasKit?.full?.glue],
  ["canvasKit.full.wasm", (assets) => assets.canvasKit?.full?.wasm],
  ["fonts.defaultSans", (assets) => assets.fonts?.defaultSans],
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
      base: {
        glue: resolve(assets.canvasKit.base.glue),
        wasm: resolve(assets.canvasKit.base.wasm),
      },
      full: {
        glue: resolve(assets.canvasKit.full.glue),
        wasm: resolve(assets.canvasKit.full.wasm),
      },
    },
    fonts: {
      defaultSans: resolve(assets.fonts.defaultSans),
    },
    workers: {
      productFrame: resolve(assets.workers.productFrame),
    },
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
