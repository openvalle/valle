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
    expect(resourceRequestCacheIdentity(request(1n, "a"))).toBe(
      resourceRequestCacheIdentity(request(9n, "a")),
    );
    expect(resourceRequestCacheIdentity(request(1n, "a", 0n))).not.toBe(
      resourceRequestCacheIdentity(request(1n, "a", 1n)),
    );
    expect(resourceRequestCacheIdentity(request(1n, "a"))).not.toBe(
      resourceRequestCacheIdentity(request(1n, "b")),
    );
    expect(resourceRequestCacheIdentity({ handle: 1n, z: "last", "𐀀": "astral", a: "first" })).toBe(
      resourceRequestCacheIdentity({ a: "first", "𐀀": "astral", z: "last", handle: 9n }),
    );
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

  test("a per-frame product stays only while consecutive frames use it", () => {
    const disposed: string[] = [];
    const cache = new BrowserResourceCache<string>({ maxEntries: 8, maxBytes: 64 });
    const frame = (lookups: string[], inserts: Array<[string, boolean]> = []) => {
      cache.beginFrame();
      for (const identity of lookups) cache.lookup(identity);
      for (const [identity, perFrame] of inserts)
        cache.insert(identity, identity, 4, () => disposed.push(identity), perFrame);
      const report = cache.finishFrame(lookups.length);
      cache.releaseFrame();
      return report;
    };
    frame(
      ["scene-1", "image"],
      [
        ["scene-1", true],
        ["image", false],
      ],
    );
    // A still scene reuses its frame.
    expect(frame(["scene-1", "image"]).hits).toBe(2);
    // Once the scene moves, its old frame leaves at the end of the first frame that does not use it.
    expect(frame(["scene-2", "image"], [["scene-2", true]]).evictions).toBe(1);
    expect(disposed).toEqual(["scene-1"]);
    // An image off screen for a frame stays in the LRU.
    frame(["scene-2"]);
    expect(disposed).toEqual(["scene-1"]);
    expect(frame(["image"]).hits).toBe(1);
    expect(disposed).toEqual(["scene-1", "scene-2"]);
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
