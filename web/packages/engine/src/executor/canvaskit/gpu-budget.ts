const MIB = 1024n * 1024n;
const DEFAULT_CACHE = 256n * MIB;
const QUANTUM = 64n * MIB;
const WASM_SIZE_MAX = 0xffffffffn;

/**
 * Ganesh's cache includes pinned F16 surfaces and internal scratch resources. Reserve retention
 * headroom when our physical pool approaches the default cache size; otherwise keep a small
 * cache. This is a retention limit, not an allocation or a hard limit on live GPU resources.
 */
export function canvasKitGpuCacheBudget(frameBytes: bigint, residentBytes: bigint): number {
  const cap = frameBytes < WASM_SIZE_MAX ? frameBytes : WASM_SIZE_MAX;
  const base = cap < DEFAULT_CACHE ? cap : DEFAULT_CACHE;
  const needed = residentBytes > base * 3n / 4n
    ? (residentBytes * 2n + QUANTUM - 1n) / QUANTUM * QUANTUM : base;
  return Number(needed < cap ? needed : cap);
}
