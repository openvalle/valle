import { expect, test } from "bun:test";
import {
  motionCompileOptionIdentity,
  prepareAudioAnalysisInputs,
} from "./audio-analysis-input.ts";

// Millions of samples must remain binary even while the Studio worker computes cache identities.
test("audio cache keys contain metadata and no PCM samples", async () => {
  const samples = new Float32Array(48_000 * 60 * 5);
  Object.defineProperty(samples, "toJSON", {
    value() {
      throw new Error("PCM must not be serialized");
    },
  });
  const key = JSON.stringify(
    await motionCompileOptionIdentity({
      audioSources: [
        {
          control: "beat",
          contentHash: "sha256:test",
          sampleRate: 48_000,
          samples,
        },
      ],
    }),
  );
  expect(key.length).toBeLessThan(150);
  expect(key).not.toContain("samples");
});

test("Studio verifies and reuses canonical Host PCM without browser decoding", async () => {
  const fetchBefore = globalThis.fetch;
  const samples = new Float32Array([0, 0.5, 0.5]);
  const bytes = new Uint8Array(samples.buffer);
  const digest = new Bun.CryptoHasher("sha256").update(bytes).digest("hex");
  const sourceHash = `sha256:${"a".repeat(64)}`;
  let downloads = 0;
  globalThis.fetch = (async (url: string) => {
    expect(url).toBe("/frozen/audio/analysis-pcm");
    downloads++;
    return new Response(bytes, { headers: {
      "X-Valle-Audio-Source": sourceHash, "X-Valle-Pcm-Digest": `sha256:${digest}`,
      "X-Valle-Sample-Rate": "48000",
    }});
  }) as unknown as typeof fetch;
  try {
    const input = { control: "beat", contentHash: sourceHash, url: "/frozen/audio" };
    const first = await prepareAudioAnalysisInputs([input]);
    expect(Array.from(first[0]!.samples)).toEqual([0, 0.5, 0.5]);
    const second = await prepareAudioAnalysisInputs([{ ...input, control: "another" }]);
    expect(second[0]!.samples).toBe(first[0]!.samples);
    expect(downloads).toBe(1);
    await expect(prepareAudioAnalysisInputs([{ ...input, contentHash: `sha256:${"f".repeat(64)}` }]))
      .rejects.toThrow("changed after preparation");
  } finally { globalThis.fetch = fetchBefore; }
});

test("binary cache identities use content digests and exclude diagnostic data origins", async () => {
  const font = new Uint8Array([1,2,3]);
  Object.defineProperty(font, "toJSON", { value() { throw new Error("font bytes must stay binary"); } });
  const first = await motionCompileOptionIdentity({ fontAliases: { "asset://caption": font }, data: { source: "first", value: { text: "hello" } } });
  const second = await motionCompileOptionIdentity({ fontAliases: { "asset://caption": new Uint8Array([1,2,3]) }, data: { source: "second", value: { text: "hello" } } });
  expect(first).toEqual(second);
  const changed = await motionCompileOptionIdentity({ fontAliases: { "asset://caption": new Uint8Array([1,2,4]) }, data: { source: "first", value: { text: "hello" } } });
  expect(changed).not.toEqual(first);
  expect(JSON.stringify(first).length).toBeLessThan(200);
});
