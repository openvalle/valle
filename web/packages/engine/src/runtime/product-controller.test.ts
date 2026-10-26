import { expect, test } from "bun:test";
import { BrowserResourceCache } from "./resource-cache.ts";
import { BrowserRuntimeBudget } from "./runtime-budget.ts";

import { BrowserValleWebPlayer } from "./product-controller.ts";

function playerFixture(): any {
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  player.planner = {
    call: async (method: string, args: unknown[]) => player.engine[method](...args),
  };
  player.runtimeBudget = new BrowserRuntimeBudget();
  player.runtimeAbort = new AbortController();
  player.memoryOwner = {};
  player.audioMemoryOwner = {};
  player.audioTail = Promise.resolve();
  player.audioPeakTasks = new Set();
  player.audioReaders = new Map();
  player.audioBuffers = new Map();
  player.runtimeBudget.register(player.memoryOwner, () => {});
  player.runtimeBudget.register(player.audioMemoryOwner, () => player.audioBuffers.clear());
  return player;
}

test("waveform peaks stream bounded PCM and preserve bucket boundaries, including empty buckets", async () => {
  const previous = globalThis.OfflineAudioContext;
  const rate = 8000,
    length = rate * 2 + 5;
  const bytes = new Uint8Array(44 + length * 2),
    header = new DataView(bytes.buffer);
  const label = (offset: number, value: string) =>
    bytes.set(new TextEncoder().encode(value), offset);
  label(0, "RIFF");
  header.setUint32(4, bytes.length - 8, true);
  label(8, "WAVEfmt ");
  header.setUint32(16, 16, true);
  header.setUint16(20, 1, true);
  header.setUint16(22, 1, true);
  header.setUint32(24, rate, true);
  header.setUint32(28, rate * 2, true);
  header.setUint16(32, 2, true);
  header.setUint16(34, 16, true);
  label(36, "data");
  header.setUint32(40, length * 2, true);
  for (let i = 0; i < length; i++) header.setInt16(44 + i * 2, ((i * 13) % 60000) - 30000, true);
  let maximum = 0;
  globalThis.OfflineAudioContext = class {
    sampleRate = rate;
    createBuffer(channels: number, count: number) {
      maximum = Math.max(maximum, count);
      const planes = Array.from({ length: channels }, () => new Float32Array(count));
      return { length: count, getChannelData: (channel: number) => planes[channel]! };
    }
  } as any;
  const player = playerFixture();
  Object.assign(player, {
    requireAsset: () => ({}),
    fetchAssetBytes: async () => bytes,
    stats: { audioPeaksDecodes: 0 },
  });
  try {
    for (const buckets of [37, length + 3]) {
      const result = await player.audioPeaks("audio", { buckets });
      const expected = new Float32Array(buckets);
      for (let b = 0; b < buckets; b++)
        for (
          let i = Math.floor((b * length) / buckets);
          i < Math.floor(((b + 1) * length) / buckets);
          i++
        ) {
          expected[b] = Math.max(expected[b]!, Math.abs(header.getInt16(44 + i * 2, true) / 32768));
        }
      expect(result.peaks).toEqual(expected);
      expect(result.durationS).toBe(length / rate);
    }
    expect(maximum).toBe(rate);
    expect(player.audioPeakTasks.size).toBe(0);
    expect(player.runtimeBudget.snapshot().kernelWorkingBytes).toBe(0);
    expect(player.runtimeBudget.snapshot().activeCpuSlots).toBe(0);
    await expect(player.audioPeaks("audio", { buckets: 0 })).rejects.toThrow("positive integer");
    player.runtimeAbort.abort();
    await expect(player.audioPeaks("audio")).rejects.toThrow();
  } finally {
    globalThis.OfflineAudioContext = previous;
  }
});

const RENDER_ID = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const SCENE_DIGEST = `sha256:${"4".repeat(64)}`;
const TOPOLOGY_DIGEST = `sha256:${"5".repeat(64)}`;

test("switching video sources retires idle decoders before decoding and preserves active sources", async () => {
  for (const allBusy of [false, true]) {
    const player = playerFixture();
    const closed: string[] = [];
    const pinned = { close: () => closed.push("pinned") };
    const previous = { close: () => closed.push("previous") };
    Object.assign(player, {
      closed: false,
      videoRings: new Map([
        ["pinned", pinned],
        ["previous", previous],
      ]),
      videoRingUsers: new Map([
        ["pinned", 1],
        ["next", 1],
      ]),
      assetUrl: () => "https://example.invalid/video",
    });
    if (allBusy) player.videoRingUsers.set("previous", 1);
    const ring = await player.videoRing({ id: "next", contentDigest: SCENE_DIGEST });
    try {
      // No frameAt call has completed yet: the first frame may still be decoding.
      expect(closed).toEqual(allBusy ? [] : ["previous"]);
      expect(player.videoRings.has("pinned")).toBe(true);
      expect(player.videoRings.get("next")).toBe(ring);
      expect(player.videoRings.size).toBe(allBusy ? 3 : 2);
      player.videoRingUsers.delete("previous");
      player.trimVideoRings();
      expect(closed).toEqual(["previous"]);
      expect(player.videoRings.size).toBe(2);
    } finally {
      ring.close();
    }
  }
});

test("preview pipeline preparation is bounded, unpublished and reused within one package", async () => {
  const player = playerFixture();
  const frames: Array<[number, boolean]> = [];
  const target = { gpu: true };
  const stop = new Error("stop after warmup");
  Object.assign(player, {
    closed: false,
    playing: false,
    playGeneration: 0,
    timeS: 0,
    renderId: RENDER_ID,
    target,
    pxScale: 1,
    renderMode: "realtime",
    renderInFlight: null,
    assetByDigest: new Map(),
    acquireTarget: () => target,
    lastFrameTimeS: () => 10,
    frameAtSeconds: () => 0,
    requireRenderReceipt: () => ({
      frameCount: 300,
      frameRate: "30/1",
      canvasWidth: 1920,
      canvasHeight: 1080,
    }),
    renderFrameDetailed: async (frame: number, _: unknown, publish: boolean) => {
      frames.push([frame, publish]);
    },
    prefetchPlanningWindow: () => [{ ready: Promise.reject(stop) }],
    prefetchAudio: async () => null,
  });
  expect(await player.play().catch((e: unknown) => e)).toBe(stop);
  expect(frames).toEqual([
    [74, false],
    [75, false],
    [149, false],
    [150, false],
    [224, false],
    [225, false],
    [299, false],
  ]);
  expect(await player.play().catch((e: unknown) => e)).toBe(stop);
  expect(frames.length).toBe(7);
  player.renderId = `sha256:${"9".repeat(64)}`;
  expect(await player.play().catch((e: unknown) => e)).toBe(stop);
  expect(frames.length).toBe(14);
  player.assetByDigest.set("video", { type: "video" });
  player.renderId = `sha256:${"6".repeat(64)}`;
  expect(await player.play().catch((e: unknown) => e)).toBe(stop);
  expect(frames.length).toBe(14);
  expect(player.currentTime()).toBe(0);
});

test("direct and buffered frames retain exclusive GPU ownership across yields", async () => {
  const player = playerFixture();
  const release = Promise.withResolvers<void>();
  const started: number[] = [];
  let active = 0,
    peak = 0;
  player.renderFrameNow = async (frame: number) => {
    started.push(frame);
    peak = Math.max(peak, ++active);
    if (frame === 0) await release.promise;
    active--;
    return { result: { frame: { index: frame } } };
  };
  const buffered = player.renderFrameDetailed(0, {}, false);
  const direct = player.renderFrame(1);
  await Bun.sleep(0);
  expect(started).toEqual([0]);
  release.resolve();
  await buffered;
  expect((await direct).frame.index).toBe(1);
  expect(started).toEqual([0, 1]);
  expect(peak).toBe(1);
});

test("a rejected frame releases GPU ownership for the next SDK request", async () => {
  const player = playerFixture();
  const error = new Error("draw failed");
  player.renderFrameNow = async (frame: number) => {
    await Bun.sleep(0);
    if (frame === 0) throw error;
    return { result: { frame: { index: frame } } };
  };
  const failed = player.renderFrame(0).catch((cause: unknown) => cause);
  const next = player.renderFrame(1);
  expect(await failed).toBe(error);
  expect((await next).frame.index).toBe(1);
});

test("GPU presentation deadlines retain surface ownership until the displayed frame is published", async () => {
  const player = playerFixture();
  const deadline = Promise.withResolvers<void>();
  const started: number[] = [],
    presented: number[] = [];
  player.renderFrameNow = async (frame: number) => {
    started.push(frame);
    return { result: { frame: { index: frame } } };
  };
  const pending = player.renderFrameDetailed(1, {}, false, async (frame: any) => {
    await deadline.promise;
    presented.push(frame.result.frame.index);
  });
  const foreign = player.renderFrame(2);
  await Bun.sleep(0);
  expect(started).toEqual([1]);
  expect(presented).toEqual([]);
  deadline.resolve();
  await pending;
  expect((await foreign).frame.index).toBe(2);
  expect(started).toEqual([1, 2]);
  expect(presented).toEqual([1]);
});

test("pausing an ahead GPU frame cancels its presentation and releases the surface for SDK rendering", async () => {
  const player = playerFixture();
  const rendered: number[] = [],
    presented: number[] = [];
  Object.assign(player, {
    closed: false,
    playing: true,
    playGeneration: 1,
    timeS: 0,
    playbackFrame: null,
    target: { gpu: true },
    requireRenderReceipt: () => ({ frameCount: 3, frameRate: "30/1" }),
    frameAtSeconds: (t: number) => Math.floor(t * 30),
    lastFrameTimeS: () => 2 / 30,
    clock: { now: () => 0 },
    prefetchPlanningWindow() {},
    presentTarget() {},
    publishInspection(completed: any) {
      presented.push(completed.result.frame.index);
    },
    async renderFrameNow(frame: number) {
      rendered.push(frame);
      return { result: { frame: { index: frame } }, width: 2, height: 2 };
    },
    pause() {
      this.playing = false;
      this.playGeneration++;
    },
  });
  const pump = player.runDirectGpuPlayback(1);
  await Bun.sleep(1);
  expect(rendered).toEqual([0, 1]);
  expect(presented).toEqual([0]);
  player.pause();
  const sdk = player.renderFrame(2);
  await pump;
  expect((await sdk).frame.index).toBe(2);
  expect(presented).toEqual([0]);
  expect(rendered).toEqual([0, 1, 2]);
});

test("the ahead GPU pump publishes its final frame once and stops at the fixed media endpoint", async () => {
  const player = playerFixture();
  let clock = 0;
  const presented: number[] = [],
    times: number[] = [];
  Object.assign(player, {
    closed: false,
    playing: true,
    playGeneration: 1,
    timeS: 0,
    playbackFrame: null,
    target: { gpu: true },
    requireRenderReceipt: () => ({ frameCount: 3, frameRate: "30/1" }),
    frameAtSeconds: (t: number) => Math.floor(t * 30),
    lastFrameTimeS: () => 2 / 30,
    clock: { now: () => clock },
    prefetchPlanningWindow() {},
    presentTarget() {},
    publishInspection(c: any) {
      presented.push(c.result.frame.index);
    },
    async renderFrameNow(frame: number) {
      clock = (frame + 1) / 30;
      return { result: { frame: { index: frame } }, width: 2, height: 2 };
    },
    pause() {
      this.playing = false;
      this.playGeneration++;
    },
    onTimeUpdate(t: number) {
      times.push(t);
    },
  });
  await player.runDirectGpuPlayback(1);
  expect(presented).toEqual([0, 1, 2]);
  expect(player.playing).toBe(false);
  expect(player.playbackFrame).toBe(2);
  expect(player.timeS).toBe(2 / 30);
  expect(times).toEqual([2 / 30]);
});

test("offscreen filter surfaces share the existing total frame budget", () => {
  const player = playerFixture();
  for (const [width, height] of [
    [320, 180],
    [1920, 1080],
    [3840, 2160],
    [7680, 4320],
  ]) {
    const root = BigInt(width!) * BigInt(height!) * 8n;
    const limits = player.framePlanningInput(10, width, height, false);
    expect(limits.maxFrameBytes).toBe(BigInt(player.runtimeBudget.memoryBytes));
    expect(limits.maxSurfaceBytes).toBe(limits.maxFrameBytes);
    expect(limits.maxSurfaceBytes).toBeGreaterThanOrEqual(root);
  }
});

test("rapid seeks wait for playback and preserve every requested frame", async () => {
  const player = playerFixture();
  const playback = Promise.withResolvers<void>();
  const renders = [Promise.withResolvers<void>(), Promise.withResolvers<void>()];
  const started: number[] = [];
  const updated: number[] = [];
  let epochs = 0;
  Object.assign(player, {
    renderInFlight: playback.promise,
    pause() {},
    requirePlanner: () => ({
      advanceEpoch() {
        epochs += 1;
      },
    }),
    lastFrameTimeS: () => 3,
    async renderTime(time: number) {
      started.push(time);
      await renders[started.length - 1]!.promise;
      return { frame: { index: time * 30 } };
    },
    onTimeUpdate(time: number) {
      updated.push(time);
    },
  });
  const first = player.seek(0.5);
  const second = player.seek(1);
  expect(started).toEqual([]);
  playback.resolve();
  await Bun.sleep(0);
  expect(started).toEqual([0.5]);
  expect(epochs).toBe(1);
  renders[0]!.resolve();
  expect(await first).toEqual({ frame: { index: 15 } });
  await Bun.sleep(0);
  expect(started).toEqual([0.5, 1]);
  expect(epochs).toBe(2);
  renders[1]!.resolve();
  expect(await second).toEqual({ frame: { index: 30 } });
  expect(updated).toEqual([0.5, 1]);
  expect(player.currentTime()).toBe(1);
  expect(player.playbackFrame).toBe(30);
  expect(player.renderInFlight).toBeNull();
});

test("a failed seek rejects its caller without blocking the next seek", async () => {
  const player = playerFixture();
  const failed = Promise.withResolvers<void>();
  const error = new Error("frame rendering failed");
  Object.assign(player, {
    renderInFlight: null,
    pause() {},
    requirePlanner: () => ({ advanceEpoch() {} }),
    lastFrameTimeS: () => 3,
    async renderTime(time: number) {
      if (time === 0.5) await failed.promise;
      return { frame: { index: time * 30 } };
    },
  });
  const first = player.seek(0.5).catch((actual: unknown) => actual);
  const second = player.seek(1);
  failed.reject(error);
  expect(await first).toBe(error);
  expect(await second).toEqual({ frame: { index: 30 } });
  expect(player.currentTime()).toBe(1);
  expect(player.renderInFlight).toBeNull();
});

test("seeking during playback prebuffering cancels the old play request quietly", async () => {
  const player = playerFixture();
  const ready = Promise.withResolvers<void>();
  let clockStarts = 0;
  Object.assign(player, {
    closed: false,
    playing: false,
    playGeneration: 0,
    timeS: 0,
    renderInFlight: null,
    lastFrameTimeS: () => 3,
    frameAtSeconds: (time: number) => time * 30,
    requireRenderReceipt: () => ({ frameRate: "30/1" }),
    prefetchPlanningWindow: () => [{ ready: ready.promise }],
    prefetchAudio: async () => null,
    clock: {
      start() {
        clockStarts += 1;
      },
    },
    pause() {
      this.playGeneration += 1;
    },
    requirePlanner: () => ({
      advanceEpoch() {
        ready.reject(new Error("epoch changed"));
      },
    }),
    renderTime: async (time: number) => ({ frame: { index: time * 30 } }),
  });
  const playing = player.play();
  await player.seek(1);
  await playing;
  expect(clockStarts).toBe(0);
  expect(player.currentTime()).toBe(1);
  expect(player.playing).toBe(false);
});

test("playback still reports an active prebuffer failure", async () => {
  const player = playerFixture();
  const error = new Error("worker failed");
  Object.assign(player, {
    closed: false,
    playing: false,
    playGeneration: 0,
    timeS: 0,
    renderInFlight: null,
    lastFrameTimeS: () => 3,
    frameAtSeconds: () => 0,
    requireRenderReceipt: () => ({ frameRate: "30/1" }),
    prefetchPlanningWindow: () => [{ ready: Promise.reject(error) }],
    prefetchAudio: async () => null,
  });
  expect(await player.play().catch((actual: unknown) => actual)).toBe(error);
});

test("failed frame planning releases its slot before admitting or loading resources", async () => {
  const player = playerFixture();
  const resources = new BrowserResourceCache();
  const error = new Error("planning failed after requests");
  let fulfilled = 0;
  let released = 0;
  Object.assign(player, {
    closed: false,
    renderId: RENDER_ID,
    pxScale: 1,
    resourceObjects: resources,
    stats: { requestBytes: 0 },
    requireRenderReceipt: () => ({ frameCount: 90, canvasWidth: 320, canvasHeight: 180 }),
    acquireTarget: () => ({ gpu: false }),
    framePlanningInput: () => ({}),
    assertActiveRenderId() {},
    requirePlanner: () => ({
      prepare: () => ({
        key: "frame",
        requests: Promise.resolve({
          renderId: RENDER_ID,
          requestPacket: new Uint8Array(),
          inspectionJson: JSON.stringify({ renderId: RENDER_ID, compositionFrame: 0, motion: [] }),
        }),
        ready: Promise.reject(error),
      }),
      release() {
        released += 1;
      },
    }),
    async fulfillRequests() {
      fulfilled++;
      resources.beginFrame();
      resources.finishFrame(0);
      return { dispose: [] };
    },
  });
  expect(await player.renderFrame(0).catch((actual: unknown) => actual)).toBe(error);
  expect(released).toBe(1);
  expect(fulfilled).toBe(0);
  expect(player.runtimeBudget.snapshot().managedBytes).toBe(0);
  expect(() => resources.dispose()).not.toThrow();
});

test("slow preview follows the media clock and still presents the final frame", async () => {
  const player = playerFixture();
  let time = 0;
  const presented: number[] = [];
  const prefetched: number[] = [];
  Object.assign(player, {
    playing: true,
    closed: false,
    playGeneration: 1,
    playbackFrame: null,
    timeS: 0,
    clock: { now: () => time },
    requireRenderReceipt: () => ({ frameRate: "30/1", frameCount: 10 }),
    frameAtSeconds: (seconds: number) => {
      expect(seconds).toBeGreaterThanOrEqual(0);
      expect(seconds).toBeLessThanOrEqual(0.3);
      return Math.floor(seconds * 30);
    },
    lastFrameTimeS: () => 9 / 30,
    prefetchPlanningWindow: (frame: number) => {
      prefetched.push(frame);
    },
    async renderFrame(frame: number) {
      presented.push(frame);
      time += 0.08;
    },
    pause() {
      this.playing = false;
    },
  });
  await player.runPlaybackPump(1);
  expect(presented).toEqual([0, 2, 4, 7, 9]);
  expect(prefetched).toEqual(presented);
  expect(player.playbackFrame).toBe(9);
  expect(player.timeS).toBe(0.3);
  expect(player.playing).toBe(false);
});

test("a late preview retires the bitmap producer before directly rendering the clock frame", async () => {
  const player = playerFixture();
  const producer = Promise.withResolvers<void>();
  const presented: number[] = [];
  let mediaTime = 4 / 30,
    bitmapsClosed = 0,
    closed = 0;
  const buffer = {
    producer: producer.promise,
    async take() {
      return {
        frame: 0,
        value: {
          bitmap: {
            close() {
              bitmapsClosed++;
            },
          },
        },
      };
    },
    close() {
      closed++;
    },
  };
  Object.assign(player, {
    playing: true,
    closed: false,
    playGeneration: 1,
    timeS: 0,
    playbackFrame: null,
    previewBuffer: buffer,
    clock: { now: () => mediaTime },
    lastFrameTimeS: () => 5 / 30,
    frameAtSeconds: (time: number) => Math.round(time * 30),
    requireRenderReceipt: () => ({ frameRate: "30/1", frameCount: 6 }),
    prefetchPlanningWindow() {},
    async renderFrame(frame: number) {
      presented.push(frame);
      mediaTime = (frame + 1) / 30;
    },
    pause() {
      this.playing = false;
    },
  });
  const pump = player.runBufferedPlayback(1, buffer);
  await Bun.sleep(0);
  expect(bitmapsClosed).toBe(1);
  expect(player.previewBuffer).toBeNull();
  expect(presented).toEqual([]);
  producer.resolve();
  await pump;
  expect(presented).toEqual([4, 5]);
  expect(closed).toBeGreaterThan(0);
  expect(player.timeS).toBe(5 / 30);
});

test("prebuffering uses the paused media position until the playback clock starts", async () => {
  const player = playerFixture();
  const made: number[] = [];
  let clockStarts = 0;
  const target = { gpu: true, canvas: { width: 2, height: 2, getContext: () => null } };
  const bitmap = globalThis.createImageBitmap;
  globalThis.createImageBitmap = (async () => ({
    close() {},
  })) as unknown as typeof createImageBitmap;
  Object.assign(player, {
    closed: false,
    playing: false,
    playGeneration: 0,
    timeS: 0,
    pxScale: 1,
    renderId: "render",
    target,
    canvas: {},
    playbackFrame: null,
    stats: {},
    assetByDigest: new Map([["video", { type: "video" }]]),
    renderMode: "realtime",
    acquireTarget: () => target,
    requireRenderReceipt: () => ({
      frameCount: 30,
      frameRate: "30/1",
      canvasWidth: 2,
      canvasHeight: 2,
    }),
    frameAtSeconds: (time: number) => Math.round(time * 30),
    lastFrameTimeS: () => 29 / 30,
    async renderFrameDetailed(
      frame: number,
      _options: unknown,
      _publish: boolean,
      afterRender?: (completed: any) => Promise<void>,
    ) {
      made.push(frame);
      const completed = {};
      await afterRender?.(completed);
      return completed;
    },
    prefetchPlanningWindow: () => [],
    prefetchAudio: async () => null,
    clock: {
      mode: "performance",
      now: () => 10,
      async start() {
        clockStarts++;
      },
    },
    teardownAudioGraph() {},
    scheduleTick() {},
    async runPlaybackPump() {},
  });
  try {
    await player.play();
    expect(clockStarts).toBe(1);
    expect(made).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
  } finally {
    player.previewBuffer?.close();
    await player.previewBuffer?.producer;
    globalThis.createImageBitmap = bitmap;
  }
});

const LEFT_DIGEST = "1111111111111111111111111111111111111111111111111111111111111111";

test("preview uses the shared media reader and caches admitted audio without requiring PCM digests", async () => {
  const player = playerFixture();
  const encoded = new Uint8Array(44 + 16);
  const view = new DataView(encoded.buffer);
  const text = (offset: number, value: string) =>
    encoded.set(new TextEncoder().encode(value), offset);
  text(0, "RIFF");
  view.setUint32(4, encoded.length - 8, true);
  text(8, "WAVEfmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 2, true);
  view.setUint32(24, 48_000, true);
  view.setUint32(28, 192_000, true);
  view.setUint16(32, 4, true);
  view.setUint16(34, 16, true);
  text(36, "data");
  view.setUint32(40, 16, true);
  for (let i = 0; i < 4; i++) {
    view.setInt16(44 + i * 4, 8192, true);
    view.setInt16(46 + i * 4, -16384, true);
  }
  let reads = 0;
  const decoded = {
    sampleRate: 48000,
    numberOfChannels: 2,
    length: 4,
    getChannelData: (channel: number) => channels[channel]!,
  };
  const channels = [new Float32Array(4), new Float32Array(4)];
  Object.assign(player, {
    audioBuffers: new Map(),
    assetByDigest: new Map([[LEFT_DIGEST, { id: "dialogue" }]]),
    async fetchAssetBytes() {
      reads++;
      return encoded;
    },
  });
  const context = {
    sampleRate: 48000,
    createBuffer: () => decoded,
  } as unknown as BaseAudioContext;
  const request = {
    digest: `sha256:${LEFT_DIGEST}`,
    sourceChannels: 2,
    startSample: 0,
    endSample: 4,
  };
  const reader = await player.audioReaderForDigest(request);
  expect(await player.audioWindow(context, request, reader)).toBe(decoded);
  expect(await player.audioWindow(context, request, reader)).toBe(decoded);
  expect(reads).toBe(1);
  expect([...channels[0]!]).toEqual([0.25, 0.25, 0.25, 0.25]);
  expect([...channels[1]!]).toEqual([-0.5, -0.5, -0.5, -0.5]);
  await expect(player.audioReaderForDigest({ ...request, sourceChannels: 1 })).rejects.toThrow(
    "channels; expected",
  );
  reader.close();
});

function pcmBuffer(channels: ReadonlyArray<ArrayLike<number>>): AudioBuffer {
  const data = channels.map((channel) => Float32Array.from(channel));
  return {
    length: data[0]!.length,
    numberOfChannels: data.length,
    getChannelData(channel: number) {
      return data[channel]!;
    },
  } as AudioBuffer;
}

test("Shader data textures use verified bytes and raw storage dimensions", async () => {
  const encoded = new Uint8Array([1, 2, 3]);
  const pixels = new Uint8Array(16);
  const image = { delete() {} };
  const player = playerFixture();
  const digest = SCENE_DIGEST.slice("sha256:".length);
  const asset = { type: "image" };
  Object.assign(player, {
    assetByDigest: new Map([[digest, asset]]),
    async fetchAssetBytes(actual: unknown, expected: string) {
      expect(actual).toBe(asset);
      expect(expected).toBe(digest);
      return encoded;
    },
    engine: {
      decode_shader_data_texture(actual: string, bytes: Uint8Array, w: number, h: number) {
        expect([actual, bytes, w, h]).toEqual([SCENE_DIGEST, encoded, 2, 1]);
        return pixels;
      },
    },
    CanvasKit: {
      ColorType: { Alpha_8: 1 },
      AlphaType: { Premul: 2 },
      ColorSpace: { SRGB: 3 },
      MakeImage(info: unknown, bytes: Uint8Array, rowBytes: number) {
        expect(info).toEqual({ width: 4, height: 4, colorType: 1, alphaType: 2, colorSpace: 3 });
        expect(bytes.byteLength).toBe(16);
        expect(rowBytes).toBe(4);
        return image;
      },
    },
  });
  const produced = await player.fulfillRequestUncached(
    {
      key: { content: SCENE_DIGEST, interpretation: { kind: "dataTexture" } },
      expected: { kind: "dataTexture", extent: { width: 2, height: 1 } },
      sample: { kind: "static" },
    },
    false,
  );
  expect(produced.object).toMatchObject({ kind: "dataTexture", image });
  expect(produced.bytes).toBe(16);
});

test("Scene3D keeps canonical ContentDigest wires at every WASM boundary", async () => {
  const calls: Array<[string, string]> = [];
  const image = { delete() {} };
  const player = playerFixture();
  Object.assign(player, {
    engine: {
      render_scene3d_request(content: string, topology: string) {
        calls.push([content, topology]);
        return new Uint8Array(8);
      },
      scene3d_frame_width(content: string) {
        expect(content).toBe(SCENE_DIGEST);
        return 1;
      },
      scene3d_frame_height(content: string) {
        expect(content).toBe(SCENE_DIGEST);
        return 1;
      },
    },
    CanvasKit: {
      ColorType: { RGBA_F16: 1 },
      AlphaType: { Premul: 1 },
      ColorSpace: { SRGB: 1 },
      MakeImage: (info: { colorType: number }, pixels: Uint8Array, rowBytes: number) => {
        expect(info.colorType).toBe(1);
        expect(pixels.byteLength).toBe(8);
        expect(rowBytes).toBe(8);
        return image;
      },
    },
    async ensureScene3dResources() {},
  });

  const canonical = JSON.stringify({ width: 1, height: 1 });
  const produced = await player.fulfillRequestUncached(
    {
      key: {
        content: SCENE_DIGEST,
        interpretation: { kind: "scene3d", topology_digest: TOPOLOGY_DIGEST },
      },
      expected: { kind: "scene3d" },
      payload: { kind: "scene3dFrame", canonical_request: canonical },
    },
    false,
    new Map([
      [SCENE_DIGEST, { canonical, topology: TOPOLOGY_DIGEST, value: JSON.parse(canonical) }],
    ]),
  );

  expect(calls).toEqual([[SCENE_DIGEST, TOPOLOGY_DIGEST]]);
  expect(produced.object.image).toBe(image);
  expect(produced.bytes).toBe(8);
});

test("Scene3D model and environment admission use frozen bytes and deduplicate registration", async () => {
  for (const kind of ["model", "environment"]) {
    const bytes = Uint8Array.from([1, 2, 3]);
    const registered: string[] = [];
    const player = playerFixture();
    Object.assign(player, {
      scene3dResourceTasks: new Map(),
      [kind === "model" ? "modelBytes" : "environmentBytes"]: new Map([
        [SCENE_DIGEST.slice("sha256:".length), bytes],
      ]),
      engine: {
        [kind === "model" ? "register_scene3d_model" : "register_scene3d_environment"](
          digest: string,
          value: Uint8Array,
        ) {
          expect(value).toBe(bytes);
          registered.push(digest);
        },
      },
    });
    await player.ensureScene3dResource(kind, SCENE_DIGEST);
    await player.ensureScene3dResource(kind, SCENE_DIGEST);
    expect(registered).toEqual([SCENE_DIGEST]);
    await expect(player.ensureScene3dResource(kind, TOPOLOGY_DIGEST)).rejects.toThrow(
      "no admitted frozen payload",
    );
  }
});

test("runtime shader fulfillment reads the Rust resource interpretation fields", async () => {
  const bytes = Uint8Array.from([1, 2, 3]);
  const player = playerFixture();
  player.shaderBytes = new Map([[`${SCENE_DIGEST.slice(7)}:${TOPOLOGY_DIGEST.slice(7)}`, bytes]]);
  const produced = await player.fulfillRequestUncached(
    {
      key: {
        content: SCENE_DIGEST,
        interpretation: { kind: "runtimeShader", abi_digest: TOPOLOGY_DIGEST },
      },
      expected: { kind: "runtimeShader" },
    },
    false,
  );
  expect(produced.object.bytes).toBe(bytes);
});

test("Scene3D textures share encoded bytes and keep color/data registrations distinct", async () => {
  const encoded = Uint8Array.from([1, 2, 3]);
  const registered: string[] = [];
  const player = playerFixture();
  Object.assign(player, {
    scene3dResourceTasks: new Map(),
    assetByDigest: new Map([[SCENE_DIGEST.slice(7), { id: "texture" }]]),
    async fetchAssetBytes() {
      return encoded;
    },
    engine: {
      register_scene3d_texture(digest: string, bytes: Uint8Array, role: string) {
        expect(bytes).toBe(encoded);
        registered.push(`${digest}:${role}`);
      },
    },
  });
  for (const role of ["color", "data", "color"])
    await player.ensureScene3dResource("texture", SCENE_DIGEST, role);
  expect(registered).toEqual([`${SCENE_DIGEST}:color`, `${SCENE_DIGEST}:data`]);
});

test("compiled audio sends one atomic bounded-window mix call and rejects malformed output", async () => {
  const player = playerFixture();
  const decoded = pcmBuffer([
    [0.1, 0.2],
    [-0.2, -0.3],
  ]);
  let calls = 0;
  const engine = {
    audio_windows_json: () =>
      JSON.stringify([
        { digest: `sha256:${LEFT_DIGEST}`, sourceChannels: 2, startSample: 0, endSample: 2 },
      ]),
    mix_audio_windows(_id: string, start: bigint, end: bigint, windows: any[]) {
      calls++;
      expect(start).toBe(0n);
      expect(end).toBe(2n);
      expect(windows).toEqual([
        {
          digest: `sha256:${LEFT_DIGEST}`,
          startSample: 0,
          left: decoded.getChannelData(0),
          right: decoded.getChannelData(1),
        },
      ]);
      return new Float32Array([0.05, -0.1, 0.1, -0.15]);
    },
  };
  Object.assign(player, {
    renderId: RENDER_ID,
    engine,
    requireCompiledAudioProgram: () => ({ sampleRate: 4, sampleCount: 2 }),
    async audioReaderForDigest() {
      return { workingBytes: () => 100 };
    },
    async audioWindow() {
      return decoded;
    },
  });
  const context = {
    sampleRate: 4,
    createBuffer: (_channels: number, length: number) =>
      pcmBuffer([new Float32Array(length), new Float32Array(length)]),
  } as unknown as BaseAudioContext;
  for (let run = 0; run < 2; run++) {
    const output = await player.renderCompiledAudioBuffer(context, 0, 2);
    expect([...output.getChannelData(0)]).toEqual([...new Float32Array([0.05, 0.1])]);
    expect([...output.getChannelData(1)]).toEqual([...new Float32Array([-0.1, -0.15])]);
  }
  expect(calls).toBe(2);
  expect(player.runtimeBudget.snapshot().managedBytes).toBe(0);
  engine.mix_audio_windows = () => new Float32Array(1);
  await expect(player.renderCompiledAudioBuffer(context, 0, 2)).rejects.toThrow("sample range");
});

test("audio decode finishing after package replacement never writes into the retired engine", async () => {
  const player = playerFixture();
  Object.assign(player, {
    renderId: RENDER_ID,
    engine: {
      audio_windows_json: () =>
        JSON.stringify([
          { digest: `sha256:${LEFT_DIGEST}`, sourceChannels: 2, startSample: 0, endSample: 1 },
        ]),
      mix_audio_windows: () => {
        throw new Error("retired engine touched");
      },
    },
    requireCompiledAudioProgram: () => ({ sampleRate: 4, sampleCount: 1 }),
    async audioReaderForDigest() {
      return { workingBytes: () => 100 };
    },
    async audioWindow() {
      player.engine = {};
      return pcmBuffer([[0.1], [-0.2]]);
    },
  });
  const context = {
    sampleRate: 4,
    createBuffer: () => pcmBuffer([[0], [0]]),
  } as unknown as BaseAudioContext;
  await expect(player.renderCompiledAudioBuffer(context, 0, 1)).rejects.toThrow("superseded");
});

test("failed render-package staging preserves the last-good runtime", async () => {
  const player = playerFixture();
  let engineFreed = 0;
  let plannerClosed = 0;
  let executorDisposed = 0;
  let resumed = 0;
  const oldEngine = {
    quantize_frame_seconds: () => 7n,
    free: () => {
      engineFreed += 1;
    },
  };
  const oldPlanner = {
    close: () => {
      plannerClosed += 1;
    },
  };
  const activeExecutor = {
    dispose: () => {
      executorDisposed += 1;
    },
  };
  Object.assign(player, {
    closed: false,
    playing: true,
    renderInFlight: null,
    renderId: RENDER_ID,
    engine: oldEngine,
    renderReceipt: { frameRate: "30/1", frameCount: 300 },
    planner: oldPlanner,
    compositor: activeExecutor,
    fixedPackageManifestJson: "old-package-manifest",
    timelineJson: "old-timeline",
    timeline: { document: {} },
    resourceManifestJson: "old-resource-manifest",
    resourceManifest: { entries: {} },
    verifiedBindingBundleJson: "old-bundle",
    assets: [],
    pause() {
      this.playing = false;
    },
    async play() {
      resumed += 1;
      this.playing = true;
    },
    async stageRenderPackage() {
      throw new Error("staged render package rejected");
    },
  });

  await expect(player.replaceRenderPackage({ timelineJson: "new-timeline" })).rejects.toThrow(
    "staged render package rejected",
  );

  expect(player.renderId).toBe(RENDER_ID);
  expect(player.engine).toBe(oldEngine);
  expect(player.planner).toBe(oldPlanner);
  expect(player.compositor).toBe(activeExecutor);
  expect(player.timelineJson).toBe("old-timeline");
  expect(player.frameAtSeconds(0.25)).toBe(7);
  expect({ engineFreed, plannerClosed, executorDisposed, resumed }).toEqual({
    engineFreed: 0,
    plannerClosed: 0,
    executorDisposed: 0,
    resumed: 1,
  });
});

test("a draft superseded while staging never replaces the active package", async () => {
  const player = playerFixture();
  const disposed: string[] = [];
  let committed = false;
  Object.assign(player, {
    closed: false,
    playing: false,
    renderInFlight: null,
    timelineJson: "last-good",
    pause() {},
    async stageRenderPackage() {
      return {
        compositor: { dispose: () => disposed.push("compositor") },
        planner: { close: () => disposed.push("planner") },
        engine: { free: () => disposed.push("engine") },
      };
    },
    commitStagedRenderPackage() {
      committed = true;
    },
  });
  expect(
    await player.replaceRenderPackage({ timelineJson: "obsolete", isCurrent: () => false }),
  ).toBeUndefined();
  expect(committed).toBe(false);
  expect(player.timelineJson).toBe("last-good");
  expect(disposed).toEqual(["compositor", "planner", "engine"]);
});

test("video source requests decode canonical rational time strings", async () => {
  const player = playerFixture();
  const asset = { type: "video" };
  const image = { delete() {} };
  const times: number[] = [];
  Object.assign(player, {
    assetByDigest: new Map([[SCENE_DIGEST.slice(7), asset]]),
    async videoImage(actual: unknown, time: number) {
      expect(actual).toBe(asset);
      times.push(time);
      return { image, texture: false, dispose() {} };
    },
  });
  const request = {
    key: { content: SCENE_DIGEST, interpretation: { kind: "video" } },
    expected: { kind: "visualFrame", extent: { width: 64, height: 36 }, pixel_layout: "rgba8" },
    sample: { kind: "sourceTime", time: "1001/30000" },
  };
  const result = await player.fulfillRequestUncached(request, false);
  expect(result.object.image).toBe(image);
  expect(times).toEqual([1001 / 30000]);
  for (const time of ["", "0/0", { numerator: 1, denominator: 30 }, 0.5]) {
    await expect(
      player.fulfillRequestUncached({ ...request, sample: { ...request.sample, time } }, false),
    ).rejects.toThrow("RationalTime");
  }
});
