import { expect, test } from "bun:test";
import type { CompiledMotion } from "valle-engine";
import { MotionCompilationCache } from "./motion-compilation-cache.ts";

test("compiler cache survives port changes, invalidates inputs and compiler bytes, and ignores corruption", async () => {
  const original = globalThis.fetch;
  const files = new Map<string, string>();
  let compilerDigest = `sha256:${"a".repeat(64)}`;
  globalThis.fetch = (async (url: string, init?: RequestInit) => {
    expect(new Headers(init?.headers).get("x-valle-token")).toBe("session");
    if (url.endsWith("/compile-cache")) return Response.json({ compilerDigest });
    const key = url.split("/").at(-1)!;
    if (init?.method === "POST") {
      files.set(key, String(init.body));
      return new Response(null, { status: 204 });
    }
    const body = files.get(key);
    return body ? new Response(body) : new Response(null, { status: 204 });
  }) as typeof fetch;
  const cache = (port: number) =>
    new MotionCompilationCache({
      url: `http://127.0.0.1:${port}/studio/compile-cache`,
      token: "session",
    });
  const compiled: CompiledMotion = {
    artifact: { component: "Card" },
    artifactDigest: `sha256:${"b".repeat(64)}`,
    warnings: [],
    sourceMap: {},
    normalizedSource: "source",
    normalizedAstDigest: "ast",
    preparedDataDigest: "data",
  };
  try {
    expect(await cache(8000).get("source + resources")).toBeNull();
    await cache(8000).put("source + resources", compiled);
    expect(await cache(9000).get("source + resources")).toEqual(compiled);
    expect(await cache(9000).get("changed source + resources")).toBeNull();
    compilerDigest = `sha256:${"c".repeat(64)}`;
    expect(await cache(9000).get("source + resources")).toBeNull();
    compilerDigest = `sha256:${"a".repeat(64)}`;
    const [key, body] = [...files][0]!;
    const corrupted = JSON.parse(body);
    corrupted.compiled.artifact.component = "Corrupted";
    files.set(key, JSON.stringify(corrupted));
    expect(await cache(9000).get("source + resources")).toBeNull();
    globalThis.fetch = (async () => {
      throw new Error("cache unavailable");
    }) as unknown as typeof fetch;
    expect(await cache(9000).get("source + resources")).toBeNull();
    await expect(cache(9000).put("source + resources", compiled)).resolves.toBeUndefined();
  } finally {
    globalThis.fetch = original;
  }
});
