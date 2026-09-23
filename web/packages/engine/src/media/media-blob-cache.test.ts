import { expect, test } from "bun:test";
import { MediaBlobCache } from "./media-blob-cache.ts";

test("duplicate media loads coalesce and failed fetches can be retried", async () => {
  const original = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = (async () => {
    calls++;
    return calls === 1 ? new Response(null, { status: 503 }) : new Response("media");
  }) as unknown as typeof fetch;
  try {
    const cache = new MediaBlobCache();
    const failed = cache.get("/asset", "identity");
    expect(cache.get("/asset", "identity")).toBe(failed);
    await expect(failed).rejects.toThrow("HTTP 503");
    const loaded = cache.get("/asset", "identity");
    expect(await (await loaded).text()).toBe("media");
    expect(cache.get("/asset", "identity")).toBe(loaded);
    expect(calls).toBe(2);
  } finally { globalThis.fetch = original; }
});

test("media cache evicts old files and invalidates changed identities", async () => {
  const original = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = (async () => { calls++; return new Response("media"); }) as unknown as typeof fetch;
  try {
    const cache = new MediaBlobCache(2, 10);
    await cache.get("/a", "v1");
    await cache.get("/b", "v1");
    await cache.get("/a", "v1"); // Touch a, so b is evicted next.
    await cache.get("/c", "v1");
    await cache.get("/b", "v1");
    expect(calls).toBe(4);
    await cache.get("/b", "v2");
    expect(calls).toBe(5);
    cache.clear();
    await cache.get("/b", "v2");
    expect(calls).toBe(6);
  } finally { globalThis.fetch = original; }
});
