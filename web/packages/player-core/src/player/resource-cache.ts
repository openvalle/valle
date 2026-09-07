import type { PackedValue } from "@valle/engine";

export interface BrowserResourceCacheLimits {
  readonly maxEntries: number;
  readonly maxBytes: number;
}

export interface BrowserResourceCacheFrameReport {
  readonly generation: number;
  readonly generationInvalidations: number;
  readonly requests: number;
  readonly hits: number;
  readonly misses: number;
  readonly insertions: number;
  readonly evictions: number;
  readonly bypasses: number;
  readonly residentEntries: number;
  readonly residentBytes: number;
}

interface CacheEntry<T> {
  readonly value: T;
  readonly bytes: number;
  readonly dispose: () => void;
  lastUsed: bigint;
  pinned: boolean;
}

interface FrameCounters {
  requests: number;
  hits: number;
  misses: number;
  insertions: number;
  evictions: number;
  bypasses: number;
}

const DEFAULT_LIMITS: BrowserResourceCacheLimits = Object.freeze({
  maxEntries: 256,
  maxBytes: 512 * 1024 * 1024,
});

/**
 * Generation-local, byte/entry-bounded LRU for browser fulfillment objects. Values are owned by
 * the cache only after `insert` returns true; eviction and generation invalidation call exactly one
 * disposer. The key is the complete canonical request identity supplied by
 * `resourceRequestCacheIdentity` plus the cache generation held by this object.
 */
export class BrowserResourceCache<T> {
  private readonly limits: BrowserResourceCacheLimits;
  private readonly entries = new Map<string, CacheEntry<T>>();
  private residentBytes = 0;
  private clock = 0n;
  private generation = 1;
  private invalidations = 0;
  private reportedInvalidations = 0;
  private frame: FrameCounters = emptyCounters();
  private frameActive = false;
  private frameFinished = false;
  private readonly framePins = new Set<string>();

  constructor(limits: BrowserResourceCacheLimits = DEFAULT_LIMITS) {
    this.limits = validateLimits(limits);
  }

  beginFrame(): void {
    if (this.frameActive) throw new Error("resource cache frame lease is already active");
    this.frame = emptyCounters();
    this.frameActive = true;
    this.frameFinished = false;
    this.framePins.clear();
  }

  lookup(identity: string): T | undefined {
    this.requireOpenFrame();
    this.frame.requests = checkedIncrement(this.frame.requests, "resource cache request count");
    const entry = this.entries.get(identity);
    if (!entry) {
      this.frame.misses = checkedIncrement(this.frame.misses, "resource cache miss count");
      return undefined;
    }
    this.frame.hits = checkedIncrement(this.frame.hits, "resource cache hit count");
    this.clock += 1n;
    entry.lastUsed = this.clock;
    entry.pinned = true;
    this.framePins.add(identity);
    return entry.value;
  }

  /** Records another handle borrowing an object already resolved by this frame. */
  recordFrameReuse(identity: string): void {
    this.requireOpenFrame();
    this.frame.requests = checkedIncrement(this.frame.requests, "resource cache request count");
    this.frame.hits = checkedIncrement(this.frame.hits, "resource cache hit count");
    const entry = this.entries.get(identity);
    if (entry) {
      this.clock += 1n;
      entry.lastUsed = this.clock;
      entry.pinned = true;
      this.framePins.add(identity);
    }
  }

  insert(identity: string, value: T, bytes: number, dispose: () => void): boolean {
    this.requireOpenFrame();
    const admittedBytes = nonNegativeSafeInteger(bytes, "resource cache object bytes");
    if (this.entries.has(identity)) {
      throw new Error("resource cache canonical identity was inserted twice");
    }
    if (admittedBytes > this.limits.maxBytes) {
      this.frame.bypasses = checkedIncrement(this.frame.bypasses, "resource cache bypass count");
      return false;
    }
    while (
      this.entries.size >= this.limits.maxEntries
      || this.residentBytes + admittedBytes > this.limits.maxBytes
    ) {
      if (!this.evictLru()) {
        this.frame.bypasses = checkedIncrement(this.frame.bypasses, "resource cache bypass count");
        return false;
      }
      this.frame.evictions = checkedIncrement(this.frame.evictions, "resource cache eviction count");
    }
    this.clock += 1n;
    this.entries.set(identity, {
      value,
      bytes: admittedBytes,
      dispose,
      lastUsed: this.clock,
      pinned: true,
    });
    this.framePins.add(identity);
    this.residentBytes = checkedAdd(this.residentBytes, admittedBytes, "resource cache resident bytes");
    this.frame.insertions = checkedIncrement(this.frame.insertions, "resource cache insertion count");
    return true;
  }

  finishFrame(expectedRequests: number): BrowserResourceCacheFrameReport {
    this.requireOpenFrame();
    const expected = nonNegativeSafeInteger(expectedRequests, "resource cache expected requests");
    if (this.frame.requests !== expected || this.frame.hits + this.frame.misses !== expected) {
      throw new Error("resource cache frame evidence is incomplete");
    }
    const generationInvalidations = this.invalidations - this.reportedInvalidations;
    this.reportedInvalidations = this.invalidations;
    this.frameFinished = true;
    return {
      generation: this.generation,
      generationInvalidations,
      requests: this.frame.requests,
      hits: this.frame.hits,
      misses: this.frame.misses,
      insertions: this.frame.insertions,
      evictions: this.frame.evictions,
      bypasses: this.frame.bypasses,
      residentEntries: this.entries.size,
      residentBytes: this.residentBytes,
    };
  }

  /** Releases cache-owned objects borrowed by the executor after that frame has stopped using them. */
  releaseFrame(): void {
    if (!this.frameActive) throw new Error("resource cache has no active frame lease");
    for (const identity of this.framePins) {
      const entry = this.entries.get(identity);
      if (entry) entry.pinned = false;
    }
    this.framePins.clear();
    this.frameActive = false;
    this.frameFinished = false;
  }

  invalidateGeneration(): void {
    if (this.frameActive) throw new Error("resource cache generation cannot change during a frame lease");
    this.disposeEntries();
    if (this.generation >= Number.MAX_SAFE_INTEGER) {
      throw new Error("resource cache generation id space exhausted");
    }
    this.generation += 1;
    this.invalidations = checkedIncrement(this.invalidations, "resource cache invalidation count");
  }

  dispose(): void {
    if (this.frameActive) throw new Error("resource cache cannot close during a frame lease");
    this.disposeEntries();
  }

  private evictLru(): boolean {
    let candidate: string | null = null;
    let lastUsed: bigint | null = null;
    for (const [identity, entry] of this.entries) {
      if (entry.pinned) continue;
      if (lastUsed === null || entry.lastUsed < lastUsed
        || (entry.lastUsed === lastUsed && identity < (candidate ?? identity))) {
        candidate = identity;
        lastUsed = entry.lastUsed;
      }
    }
    if (candidate === null) return false;
    const entry = this.entries.get(candidate);
    if (!entry) throw new Error("resource cache LRU candidate disappeared");
    this.entries.delete(candidate);
    this.residentBytes -= entry.bytes;
    entry.dispose();
    return true;
  }

  private disposeEntries(): void {
    const entries = [...this.entries.values()];
    this.entries.clear();
    this.residentBytes = 0;
    this.clock = 0n;
    let firstError: unknown = null;
    for (const entry of entries) {
      try {
        entry.dispose();
      } catch (error) {
        firstError ??= error;
      }
    }
    if (firstError !== null) throw firstError;
  }

  private requireOpenFrame(): void {
    if (!this.frameActive || this.frameFinished) {
      throw new Error("resource cache operation is outside an open frame lease");
    }
  }
}

/** Complete request identity, deliberately excluding only the frame-local handle. */
export function resourceRequestCacheIdentity(request: PackedValue): string {
  if (!isRecord(request)) throw new Error("resource cache request must be an object");
  const { handle: _handle, ...identity } = request;
  return encodePacked(identity);
}

function encodePacked(value: PackedValue): string {
  if (value === null) return "n";
  if (typeof value === "boolean") return value ? "b1" : "b0";
  if (typeof value === "bigint") return `i${value.toString(10)};`;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error("resource cache identity contains non-finite number");
    return `f${Object.is(value, -0) ? "-0" : value.toString()};`;
  }
  if (typeof value === "string") return `s${utf8Length(value)}:${value}`;
  if (Array.isArray(value)) return `a${value.length}[${value.map(encodePacked).join("")}]`;
  const entries = Object.entries(value).sort(([left], [right]) => compareUtf8(left, right));
  return `o${entries.length}{${entries
    .map(([key, child]) => `${encodePacked(key)}${encodePacked(child)}`)
    .join("")}}`;
}

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function compareUtf8(left: string, right: string): number {
  const leftBytes = new TextEncoder().encode(left);
  const rightBytes = new TextEncoder().encode(right);
  const length = Math.min(leftBytes.length, rightBytes.length);
  for (let index = 0; index < length; index += 1) {
    const order = leftBytes[index]! - rightBytes[index]!;
    if (order !== 0) return order;
  }
  return leftBytes.length - rightBytes.length;
}

function isRecord(value: PackedValue): value is { readonly [key: string]: PackedValue } {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function validateLimits(limits: BrowserResourceCacheLimits): BrowserResourceCacheLimits {
  return Object.freeze({
    maxEntries: positiveSafeInteger(limits.maxEntries, "resource cache maxEntries"),
    maxBytes: positiveSafeInteger(limits.maxBytes, "resource cache maxBytes"),
  });
}

function emptyCounters(): FrameCounters {
  return { requests: 0, hits: 0, misses: 0, insertions: 0, evictions: 0, bypasses: 0 };
}

function positiveSafeInteger(value: number, label: string): number {
  if (!Number.isSafeInteger(value) || value <= 0) throw new Error(`${label} must be positive`);
  return value;
}

function nonNegativeSafeInteger(value: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error(`${label} must be non-negative`);
  return value;
}

function checkedIncrement(value: number, label: string): number {
  return checkedAdd(value, 1, label);
}

function checkedAdd(left: number, right: number, label: string): number {
  const result = left + right;
  if (!Number.isSafeInteger(result)) throw new Error(`${label} overflowed`);
  return result;
}
