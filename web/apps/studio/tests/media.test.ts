import { afterEach, beforeEach, expect, test } from "bun:test";
import { invalidate, peaks } from "../src/media.ts";

const originalFetch = globalThis.fetch;
const audio = Object.getOwnPropertyDescriptor(globalThis, "OfflineAudioContext");
let requests: { url: string; signal: AbortSignal; finish: (response: Response) => void }[];

beforeEach(() => {
  requests = [];
  globalThis.fetch = ((url: string, init: RequestInit) =>
    new Promise<Response>((finish) => {
      // Ignore abort here to exercise a decode which was already in flight when invalidated.
      requests.push({ url, signal: init.signal as AbortSignal, finish });
    })) as typeof fetch;
  Object.defineProperty(globalThis, "OfflineAudioContext", {
    configurable: true,
    value: class {
      async decodeAudioData(data: ArrayBuffer) {
        return {
          sampleRate: 50,
          length: 1,
          numberOfChannels: 1,
          getChannelData: () => new Float32Array([new Uint8Array(data)[0] / 100]),
        };
      }
    },
  });
});

afterEach(() => {
  invalidate();
  globalThis.fetch = originalFetch;
  if (audio) Object.defineProperty(globalThis, "OfflineAudioContext", audio);
  else Reflect.deleteProperty(globalThis, "OfflineAudioContext");
});

async function finish(index: number, amplitude: number) {
  requests[index].finish(new Response(new Uint8Array([amplitude])));
  await Bun.sleep(0);
}

test("shared waveform requests notify both the media browser and timeline", async () => {
  let first = 0,
    second = 0;
  expect(peaks("/sound.wav", () => first++)).toBeUndefined();
  expect(peaks("/sound.wav", () => second++)).toBeUndefined();
  expect(requests).toHaveLength(1);
  await finish(0, 40);
  expect([first, second]).toEqual([1, 1]);
  expect(peaks("/sound.wav", () => {})![0]).toBeCloseTo(0.4);
});

test("a stale decode cannot overwrite a replacement or notify its old subscriber", async () => {
  let old = 0,
    current = 0;
  peaks("/sound.wav", () => old++);
  invalidate("/sound.wav");
  expect(requests[0].signal.aborted).toBe(true);
  peaks("/sound.wav", () => current++);
  expect(requests[1].url).not.toBe(requests[0].url);
  await finish(1, 80);
  await finish(0, 20);
  expect([old, current]).toEqual([0, 1]);
  expect(peaks("/sound.wav", () => {})![0]).toBeCloseTo(0.8);
});

test("project changes discard all completed and pending media", async () => {
  peaks("/a.wav", () => {});
  peaks("/b.wav", () => {});
  await finish(0, 20);
  invalidate();
  expect(requests.every((request) => request.signal.aborted)).toBe(true);
  expect(peaks("/a.wav", () => {})).toBeUndefined();
  expect(peaks("/b.wav", () => {})).toBeUndefined();
  expect(requests).toHaveLength(4);
});
