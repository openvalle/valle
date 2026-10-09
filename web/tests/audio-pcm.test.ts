import { expect, test } from "bun:test";
import { initSync, ProductEngine, prepare_preview_package } from "../packages/engine/generated/web/valle_engine.js";
import { preparePreviewPackageWithWasm, type PreviewResourceInput } from "../packages/engine/src/compiler.ts";
import type { Timeline } from "../packages/engine/src/timeline.ts";

initSync({ module: await Bun.file(new URL("../packages/engine/generated/web/valle_engine_bg.wasm", import.meta.url)).arrayBuffer() });
const rate = 48_000;

function fixture(tracks: NonNullable<Timeline["tracks"]["audio"]>, layouts: Array<"mono" | "stereo">, sourceDuration = 2) {
  const digests = layouts.map((_, index) => `sha256:${new Bun.CryptoHasher("sha256").update(`pcm:${index}`).digest("hex")}`);
  const resources = Object.fromEntries(layouts.map((_, index) => [`s${index}`, `s${index}.wav`]));
  const inputs: PreviewResourceInput[] = layouts.map((layout, index) => {
    const descriptor = { duration: `${sourceDuration}/1`, timeBase: `1/${rate}`, presentationIndexDigest: digests[index],
      sampleRate: rate, channelLayout: layout, audioStream: 0 };
    return { id: `resource:s${index}`, entry: { kind: "audio", digest: digests[index], descriptor },
      facts: { kind: "audio", descriptor, temporalFootprint: { pastSamples: 0, futureSamples: 0 } } };
  });
  const prepared = preparePreviewPackageWithWasm({ prepare_preview_package }, {
    authorTimeline: { canvas: { width: 2, height: 2, fps: 30 }, resources,
      tracks: { visual: [{ clips: [{ kind: "solid", color: "#000000", start: 0, duration: 2 }] }], audio: tracks } },
    motionInstances: [], resourceInputs: inputs,
  });
  const engine = new ProductEngine();
  const open = () => JSON.parse(engine.open_fixed_package(prepared.fixedPackageManifestJson, prepared.timelineJson,
    prepared.resourceManifestJson, prepared.verifiedBindingBundleJson)).renderId as string;
  return { engine, id: open(), open, digests };
}

test("actual WASM mixes simultaneous mono/stereo sources with exact gain, pan and clamping", () => {
  const { engine, id, digests } = fixture([
    { clips: [{ src: "s0", start: 0, duration: 2, gain: 0.5, pan: -1 }] },
    { clips: [{ src: "s1", start: 0, duration: 2, gain: 0.5, pan: 1 }] },
    { clips: [{ src: "s2", start: 0, duration: 1, gain: 2 }] },
  ], ["mono", "stereo", "mono"]);
  try {
    engine.register_audio_pcm(id, digests[0]!, new Float32Array(rate * 2).fill(0.5), new Float32Array(0));
    engine.register_audio_pcm(id, digests[1]!, new Float32Array(rate * 2).fill(0.75), new Float32Array(rate * 2).fill(-0.25));
    engine.register_audio_pcm(id, digests[2]!, new Float32Array(rate * 2).fill(0.5), new Float32Array(0));
    expect(JSON.parse(engine.audio_sources_json(id, 0n, 1n))).toHaveLength(3);
    expect([...engine.mix_audio_pcm(id, 0n, 1n)]).toEqual([1, 0.875]);
    // A block with all three sources must retain all of them despite the ordinary two-source cache.
    expect(digests.every((digest) => engine.has_audio_pcm(id, digest))).toBe(true);
    expect([...engine.mix_audio_pcm(id, 48000n, 48001n)]).toEqual([0.25, -0.125]);
    expect(digests.filter((digest) => engine.has_audio_pcm(id, digest))).toHaveLength(2);
  } finally { engine.free(); }
});

test("actual WASM preserves trim, loop, curves, silence and random-seek/chunk boundaries", () => {
  const { engine, id, digests } = fixture([{ clips: [{ src: "s0", start: 0.25, duration: 1.5,
    trimStart: 0.5, rate: 2, end: "loop", gain: { keyframes: [[0, 0.5], [0.5, 1]] } }] }], ["mono"], 1);
  const source = Float32Array.from({ length: rate }, (_, index) => [0.1, 0.2, 0.3, 0.4][Math.floor(index / 12000)]!);
  try {
    engine.register_audio_pcm(id, digests[0]!, source, new Float32Array(0));
    const sample = (index: number) => [...engine.mix_audio_pcm(id, BigInt(index), BigInt(index + 1))];
    expect(sample(0)).toEqual([0, 0]);
    expect(sample(12000)).toEqual([Math.fround(source[24000]! * 0.5), Math.fround(source[24000]! * 0.5)]);
    expect(sample(24000)).toEqual([Math.fround(source[0]! * 0.75), Math.fround(source[0]! * 0.75)]);
    expect(sample(36000)).toEqual([source[24000], source[24000]]);
    expect(sample(84000)).toEqual([0, 0]);
    const whole = new Float32Array(rate * 4);
    for (let start = 0; start < rate * 2; start += 8192) {
      whole.set(engine.mix_audio_pcm(id, BigInt(start), BigInt(Math.min(start + 8192, rate * 2))), start * 2);
    }
    const seek = engine.mix_audio_pcm(id, 47000n, 53000n);
    expect(seek).toEqual(whole.slice(94000, 106000));
  } finally { engine.free(); }
});

test("actual WASM rejects invalid fulfillment and resets PCM even for the same render identity", () => {
  const { engine, id, open, digests } = fixture([{ clips: [{ src: "s0", start: 0, duration: 2 }] }], ["stereo"]);
  const plane = new Float32Array(rate * 2).fill(0.25);
  try {
    expect(() => engine.mix_audio_pcm(id, 0n, 1n)).toThrow("no decoded PCM");
    expect(() => engine.audio_sources_json(id, -1n, 1n)).toThrow("audio_range");
    expect(() => engine.mix_audio_pcm(id, 0n, 8193n)).toThrow("audio_range");
    expect(() => engine.register_audio_pcm(id, digests[0]!, plane, new Float32Array(0))).toThrow("channel layout");
    expect(() => engine.register_audio_pcm(id, `sha256:${"0".repeat(64)}`, plane, plane)).toThrow("not admitted");
    engine.register_audio_pcm(id, digests[0]!, new Float32Array([NaN]), new Float32Array([0]));
    expect(() => engine.mix_audio_pcm(id, 0n, 1n)).toThrow("non-finite");
    engine.register_audio_pcm(id, digests[0]!, new Float32Array([0]), new Float32Array([0]));
    expect(() => engine.mix_audio_pcm(id, 1n, 2n)).toThrow("outside decoded PCM");
    engine.register_audio_pcm(id, digests[0]!, plane, plane);
    expect([...engine.mix_audio_pcm(id, 0n, 1n)]).toEqual([0.25, 0.25]);
    expect(open()).toBe(id);
    expect(engine.has_audio_pcm(id, digests[0]!)).toBe(false);
    engine.reset();
    expect(() => engine.has_audio_pcm(id, digests[0]!)).toThrow("render_closed");
  } finally { engine.free(); }
});
