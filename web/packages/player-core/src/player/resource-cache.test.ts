import { describe, expect, test } from "bun:test";
import { BrowserResourceCache, resourceRequestCacheIdentity } from "./resource-cache.ts";

function request(handle: bigint, content: string, sample: bigint = 0n) {
  return {
    handle,
    key: { content, interpretation: { kind: "visual", alpha: "opaque" } },
    sample: { kind: "sourceTime", time: { numerator: sample, denominator: 30n } },
    expected: { kind: "visualFrame", extent: { width: 1n, height: 1n }, pixelLayout: "rgba8" },
    payload: null,
  } as const;
}

describe("browser resource cache", () => {
  test("identity excludes only the frame-local handle", () => {
    expect(resourceRequestCacheIdentity(request(1n, "a")))
      .toBe(resourceRequestCacheIdentity(request(9n, "a")));
    expect(resourceRequestCacheIdentity(request(1n, "a", 0n)))
      .not.toBe(resourceRequestCacheIdentity(request(1n, "a", 1n)));
    expect(resourceRequestCacheIdentity(request(1n, "a")))
      .not.toBe(resourceRequestCacheIdentity(request(1n, "b")));
    expect(resourceRequestCacheIdentity({ handle: 1n, z: "last", "𐀀": "astral", a: "first" }))
      .toBe(resourceRequestCacheIdentity({ a: "first", "𐀀": "astral", z: "last", handle: 9n }));
  });

  test("LRU, frame pinning, byte capacity and generation disposal are deterministic", () => {
    const disposed: string[] = [];
    const cache = new BrowserResourceCache<string>({ maxEntries: 2, maxBytes: 12 });
    cache.beginFrame();
    expect(cache.lookup("a")).toBeUndefined();
    expect(cache.insert("a", "A", 4, () => disposed.push("a"))).toBeTrue();
    expect(cache.lookup("b")).toBeUndefined();
    expect(cache.insert("b", "B", 4, () => disposed.push("b"))).toBeTrue();
    expect(cache.finishFrame(2)).toEqual({
      generation: 1,
      generationInvalidations: 0,
      requests: 2,
      hits: 0,
      misses: 2,
      insertions: 2,
      evictions: 0,
      bypasses: 0,
      residentEntries: 2,
      residentBytes: 8,
    });
    cache.releaseFrame();

    cache.beginFrame();
    expect(cache.lookup("a")).toBe("A");
    expect(cache.lookup("c")).toBeUndefined();
    expect(cache.insert("c", "C", 8, () => disposed.push("c"))).toBeTrue();
    expect(disposed).toEqual(["b"]);
    expect(cache.finishFrame(2)).toEqual({
      generation: 1,
      generationInvalidations: 0,
      requests: 2,
      hits: 1,
      misses: 1,
      insertions: 1,
      evictions: 1,
      bypasses: 0,
      residentEntries: 2,
      residentBytes: 12,
    });
    cache.releaseFrame();

    cache.invalidateGeneration();
    expect(disposed).toEqual(["b", "a", "c"]);
    cache.beginFrame();
    expect(cache.lookup("large")).toBeUndefined();
    expect(cache.insert("large", "L", 13, () => disposed.push("large"))).toBeFalse();
    expect(cache.finishFrame(1)).toEqual({
      generation: 2,
      generationInvalidations: 1,
      requests: 1,
      hits: 0,
      misses: 1,
      insertions: 0,
      evictions: 0,
      bypasses: 1,
      residentEntries: 0,
      residentBytes: 0,
    });
    cache.releaseFrame();
    expect(disposed).not.toContain("large");
  });

  test("objects borrowed by a frame are bypassed instead of evicted", () => {
    const disposed: string[] = [];
    const cache = new BrowserResourceCache<string>({ maxEntries: 1, maxBytes: 4 });
    cache.beginFrame();
    expect(cache.lookup("a")).toBeUndefined();
    expect(cache.insert("a", "A", 4, () => disposed.push("a"))).toBeTrue();
    expect(cache.lookup("b")).toBeUndefined();
    expect(cache.insert("b", "B", 4, () => disposed.push("b"))).toBeFalse();
    expect(disposed).toEqual([]);
    expect(cache.finishFrame(2).bypasses).toBe(1);
    cache.releaseFrame();
    cache.invalidateGeneration();
    expect(disposed).toEqual(["a"]);
  });
});
