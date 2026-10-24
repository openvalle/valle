const MIB = 1024n * 1024n;
const QUANTUM = 64n * MIB;
const SCRATCH_HEADROOM = 128n * MIB;
const WASM_SIZE_MAX = 0xffffffffn;

/**
 * Ganesh's cache includes pinned F16 surfaces and internal scratch resources. Retain a fixed
 * scratch headroom above the pinned surfaces: released working surfaces, layers and atlases stay
 * reusable within it, and Ganesh purges the least recently used beyond it. This is a retention
 * limit, not an allocation or a hard limit on live GPU resources.
 */
export function canvasKitGpuCacheBudget(frameBytes: bigint, pinnedBytes: bigint): number {
  const cap = frameBytes < WASM_SIZE_MAX ? frameBytes : WASM_SIZE_MAX;
  const needed = ((pinnedBytes + SCRATCH_HEADROOM + QUANTUM - 1n) / QUANTUM) * QUANTUM;
  return Number(needed < cap ? needed : cap);
}
