import { expect, test } from "bun:test";
import { createMotionFontLoader } from "./fonts.ts";

test("font URLs load lazily once, preserve stack order and share duplicate requests", async () => {
  const original = globalThis.fetch;
  const requested: string[] = [];
  globalThis.fetch = (async (url: string | URL | Request) => {
    requested.push(String(url));
    return new Response(new Uint8Array([requested.length]));
  }) as unknown as typeof fetch;
  try {
    const load = createMotionFontLoader(["brand.ttf", new Uint8Array([9]), "brand.ttf", "fallback.otf"], "https://fonts.example.test/assets/");
    expect(requested).toEqual([]);
    const first = load();
    expect(load()).toBe(first);
    const result = JSON.parse(await first);
    expect(result.map((font: { bytesBase64: string }) => atob(font.bytesBase64).charCodeAt(0))).toEqual([1, 9, 1, 2]);
    expect(requested).toEqual(["https://fonts.example.test/assets/brand.ttf", "https://fonts.example.test/assets/fallback.otf"]);
    expect(await load()).toBe(await first);
    expect(requested.length).toBe(2);
  } finally { globalThis.fetch = original; }
});

test("failed font requests can be retried and input bytes are snapshotted", async () => {
  const original = globalThis.fetch;
  let attempts = 0;
  globalThis.fetch = (async () => ++attempts === 1
    ? new Response("missing", { status: 404 })
    : new Response(new Uint8Array([2]))) as unknown as typeof fetch;
  try {
    const bytes = new Uint8Array([1]);
    const load = createMotionFontLoader([bytes, "brand.ttf"], "https://fonts.example.test/");
    bytes[0] = 99;
    await expect(load()).rejects.toThrow("font request failed (404)");
    expect(JSON.parse(await load())[0].bytesBase64).toBe("AQ==");
    expect(attempts).toBe(2);
  } finally { globalThis.fetch = original; }
});
