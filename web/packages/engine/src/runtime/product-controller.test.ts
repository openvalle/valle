import { expect, test } from "bun:test";
import { BrowserResourceCache } from "./resource-cache.ts";

import { BrowserValleWebPlayer } from "./product-controller.ts";

const RENDER_ID = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const SCENE_DIGEST = `sha256:${"4".repeat(64)}`;
const TOPOLOGY_DIGEST = `sha256:${"5".repeat(64)}`;

test("rapid seeks wait for playback and preserve every requested frame", async () => {
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  const playback = Promise.withResolvers<void>();
  const renders = [Promise.withResolvers<void>(), Promise.withResolvers<void>()];
  const started: number[] = [];
  const updated: number[] = [];
  let epochs = 0;
  Object.assign(player, {
    renderInFlight: playback.promise,
    pause() {},
    requirePlanner: () => ({ advanceEpoch() { epochs += 1; } }),
    lastFrameTimeS: () => 3,
    async renderTime(time: number) {
      started.push(time);
      await renders[started.length - 1]!.promise;
      return { frame: { index: time * 30 } };
    },
    onTimeUpdate(time: number) { updated.push(time); },
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
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
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
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  const ready = Promise.withResolvers<void>();
  let clockStarts = 0;
  Object.assign(player, {
    closed: false, playing: false, playGeneration: 0, timeS: 0, renderInFlight: null,
    lastFrameTimeS: () => 3,
    frameAtSeconds: (time: number) => time * 30,
    requireRenderReceipt: () => ({ frameRate: "30/1" }),
    prefetchPlanningWindow: () => [{ ready: ready.promise }],
    prefetchAudio: async () => null,
    clock: { start() { clockStarts += 1; } },
    pause() { this.playGeneration += 1; },
    requirePlanner: () => ({ advanceEpoch() { ready.reject(new Error("epoch changed")); } }),
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
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  const error = new Error("worker failed");
  Object.assign(player, {
    closed: false, playing: false, playGeneration: 0, timeS: 0, renderInFlight: null,
    lastFrameTimeS: () => 3,
    frameAtSeconds: () => 0,
    requireRenderReceipt: () => ({ frameRate: "30/1" }),
    prefetchPlanningWindow: () => [{ ready: Promise.reject(error) }],
    prefetchAudio: async () => null,
  });
  expect(await player.play().catch((actual: unknown) => actual)).toBe(error);
});

test("failed frame planning releases resources that finished loading", async () => {
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  const resources = new BrowserResourceCache();
  const error = new Error("planning failed after requests");
  let disposed = 0;
  let released = 0;
  Object.assign(player, {
    closed: false, renderId: RENDER_ID, pxScale: 1, resourceObjects: resources,
    stats: { requestBytes: 0 },
    requireRenderReceipt: () => ({ frameCount: 90, canvasWidth: 320, canvasHeight: 180 }),
    acquireTarget: () => ({ gpu: false }),
    framePlanningInput: () => ({}),
    assertActiveRenderId() {},
    requirePlanner: () => ({
      prepare: () => ({
        key: "frame",
        requests: Promise.resolve({
          renderId: RENDER_ID, requestPacket: new Uint8Array(),
          inspectionJson: JSON.stringify({ renderId: RENDER_ID, compositionFrame: 0, motion: [] }),
        }),
        ready: Promise.reject(error),
      }),
      release() { released += 1; },
    }),
    async fulfillRequests() {
      resources.beginFrame();
      resources.finishFrame(0);
      return { dispose: [() => { disposed += 1; }] };
    },
  });
  expect(await player.renderFrame(0).catch((actual: unknown) => actual)).toBe(error);
  expect(released).toBe(1);
  expect(disposed).toBe(1);
  expect(() => resources.dispose()).not.toThrow();
});

test("slow preview follows the media clock and still presents the final frame", async () => {
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
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
    prefetchPlanningWindow: (frame: number) => { prefetched.push(frame); },
    async renderFrame(frame: number) { presented.push(frame); time += 0.08; },
    pause() { this.playing = false; },
  });
  await player.runPlaybackPump(1);
  expect(presented).toEqual([0, 2, 4, 7, 9]);
  expect(prefetched).toEqual(presented);
  expect(player.playbackFrame).toBe(9);
  expect(player.timeS).toBe(0.3);
  expect(player.playing).toBe(false);
});

const LEFT_DIGEST = "1111111111111111111111111111111111111111111111111111111111111111";
const RIGHT_DIGEST = "2222222222222222222222222222222222222222222222222222222222222222";

test("preview uses the shared media reader and caches admitted audio without requiring PCM digests", async () => {
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  const encoded = new Uint8Array(44 + 16);
  const view = new DataView(encoded.buffer);
  const text = (offset: number, value: string) => encoded.set(new TextEncoder().encode(value), offset);
  text(0, "RIFF"); view.setUint32(4, encoded.length - 8, true); text(8, "WAVEfmt ");
  view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 2, true);
  view.setUint32(24, 48_000, true); view.setUint32(28, 192_000, true); view.setUint16(32, 4, true);
  view.setUint16(34, 16, true); text(36, "data"); view.setUint32(40, 16, true);
  for (let i = 0; i < 4; i++) { view.setInt16(44 + i * 4, 8192, true); view.setInt16(46 + i * 4, -16384, true); }
  let reads = 0;
  const decoded = { sampleRate: 48000, numberOfChannels: 2, length: 4,
    getChannelData: (channel: number) => channels[channel]! };
  const channels = [new Float32Array(4), new Float32Array(4)];
  Object.assign(player, {
    audioBuffers: new Map(),
    assetByDigest: new Map([[LEFT_DIGEST, { id: "dialogue" }]]),
    async fetchAssetBytes() { reads++; return encoded; },
  });
  const context = {
    sampleRate: 48000,
    createBuffer: () => decoded,
  } as unknown as BaseAudioContext;
  expect(await player.audioBufferForDigest(context, `sha256:${LEFT_DIGEST}`, 2)).toBe(decoded);
  expect(await player.audioBufferForDigest(context, `sha256:${LEFT_DIGEST}`, 2)).toBe(decoded);
  expect(reads).toBe(1);
  expect([...channels[0]!]).toEqual([0.25, 0.25, 0.25, 0.25]);
  expect([...channels[1]!]).toEqual([-0.5, -0.5, -0.5, -0.5]);
  await expect(player.audioBufferForDigest(context, `sha256:${LEFT_DIGEST}`, 1)).rejects.toThrow("channels; expected");
  decoded.numberOfChannels = 1;
  player.audioBuffers.clear();
  await expect(player.audioBufferForDigest(context, `sha256:${LEFT_DIGEST}`, 2)).rejects.toThrow("1 channels; expected");
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
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  const digest = SCENE_DIGEST.slice("sha256:".length);
  const asset = { type: "image" };
  Object.assign(player, {
    assetByDigest: new Map([[digest, asset]]),
    async fetchAssetBytes(actual: unknown, expected: string) {
      expect(actual).toBe(asset); expect(expected).toBe(digest); return encoded;
    },
    engine: { decode_shader_data_texture(actual: string, bytes: Uint8Array, w: number, h: number) {
      expect([actual, bytes, w, h]).toEqual([SCENE_DIGEST, encoded, 2, 1]); return pixels;
    } },
    CanvasKit: {
      ColorType: { Alpha_8: 1 }, AlphaType: { Premul: 2 }, ColorSpace: { SRGB: 3 },
      MakeImage(info: unknown, bytes: Uint8Array, rowBytes: number) {
        expect(info).toEqual({ width: 4, height: 4, colorType: 1, alphaType: 2, colorSpace: 3 });
        expect(bytes.byteLength).toBe(16); expect(rowBytes).toBe(4); return image;
      },
    },
  });
  const produced = await player.fulfillRequestUncached({
    key: { content: SCENE_DIGEST, interpretation: { kind: "dataTexture" } },
    expected: { kind: "dataTexture", extent: { width: 2, height: 1 } }, sample: { kind: "static" },
  }, false);
  expect(produced.object).toMatchObject({ kind: "dataTexture", image });
  expect(produced.bytes).toBe(16);
});

test("Scene3D keeps canonical ContentDigest wires at every WASM boundary", async () => {
  const calls: Array<[string, string]> = [];
  const image = { delete() {} };
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
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
      MakeImage: (info: {colorType:number}, pixels: Uint8Array, rowBytes: number) => {
        expect(info.colorType).toBe(1);
        expect(pixels.byteLength).toBe(8);
        expect(rowBytes).toBe(8);
        return image;
      },
    },
    async ensureScene3dResources() {},
  });

  const produced = await player.fulfillRequestUncached({
    key: {
      content: SCENE_DIGEST,
      interpretation: { kind: "scene3d", topology_digest: TOPOLOGY_DIGEST },
    },
    expected: { kind: "scene3d" },
    payload: { kind: "scene3dFrame", canonical_request: [1, 2, 3] },
  }, false);

  expect(calls).toEqual([[SCENE_DIGEST, TOPOLOGY_DIGEST]]);
  expect(produced.object.image).toBe(image);
  expect(produced.bytes).toBe(8);
});

test("Scene3D model and environment admission use frozen bytes and deduplicate registration", async () => {
  for (const kind of ["model", "environment"]) {
  const bytes = Uint8Array.from([1, 2, 3]);
  const registered: string[] = [];
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  Object.assign(player, {
    scene3dResourceTasks: new Map(),
    [kind === "model" ? "modelBytes" : "environmentBytes"]: new Map([[SCENE_DIGEST.slice("sha256:".length), bytes]]),
    engine: { [kind === "model" ? "register_scene3d_model" : "register_scene3d_environment"](digest: string, value: Uint8Array) {
      expect(value).toBe(bytes); registered.push(digest);
    } },
  });
  await player.ensureScene3dResource(kind, SCENE_DIGEST);
  await player.ensureScene3dResource(kind, SCENE_DIGEST);
  expect(registered).toEqual([SCENE_DIGEST]);
  await expect(player.ensureScene3dResource(kind, TOPOLOGY_DIGEST)).rejects.toThrow("no admitted frozen payload");
  }
});

test("runtime shader fulfillment reads the Rust resource interpretation fields", async () => {
  const bytes = Uint8Array.from([1, 2, 3]);
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  player.shaderBytes = new Map([[`${SCENE_DIGEST.slice(7)}:${TOPOLOGY_DIGEST.slice(7)}`, bytes]]);
  const produced = await player.fulfillRequestUncached({
    key: {content: SCENE_DIGEST, interpretation: {kind: "runtimeShader", abi_digest: TOPOLOGY_DIGEST}},
    expected: {kind: "runtimeShader"},
  }, false);
  expect(produced.object.bytes).toBe(bytes);
});

test("Scene3D textures share encoded bytes and keep color/data registrations distinct", async () => {
  const encoded=Uint8Array.from([1,2,3]);
  const registered:string[]=[];
  const player=Object.create(BrowserValleWebPlayer.prototype) as any;
  Object.assign(player,{
    scene3dResourceTasks:new Map(),
    assetByDigest:new Map([[SCENE_DIGEST.slice(7),{id:"texture"}]]),
    async fetchAssetBytes(){return encoded;},
    engine:{register_scene3d_texture(digest:string,bytes:Uint8Array,role:string){expect(bytes).toBe(encoded);registered.push(`${digest}:${role}`);}},
  });
  for (const role of ["color","data","color"]) await player.ensureScene3dResource("texture",SCENE_DIGEST,role);
  expect(registered).toEqual([`${SCENE_DIGEST}:color`,`${SCENE_DIGEST}:data`]);
});

test("compiled audio registers canonical source planes once and accepts only bounded PCM output", async () => {
  const declared: string[] = [];
  const registered: string[] = [];
  const planes = [new Float32Array([0.1, 0.2]), new Float32Array([-0.2, -0.3])];
  const decoded = pcmBuffer(planes);
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  const engine = {
    audio_sources_json: () => JSON.stringify([{ sourceIndex: 0, digest: `sha256:${LEFT_DIGEST}`, sourceChannels: 2 }]),
    has_audio_pcm: () => registered.length > 0,
    register_audio_pcm(_id: string, digest: string, left: Float32Array, right: Float32Array) {
      registered.push(digest); expect(left).toBe(decoded.getChannelData(0)); expect(right).toBe(decoded.getChannelData(1));
    },
    mix_audio_pcm: (_id: string, start: bigint, end: bigint) => {
      expect(start).toBe(0n); expect(end).toBe(2n);
      return new Float32Array([0.05, -0.1, 0.1, -0.15]);
    },
  };
  Object.assign(player, {
    renderId: RENDER_ID, engine, audioBuffers: new Map(),
    requireCompiledAudioProgram: () => ({ sampleRate: 4, sampleCount: 2 }),
    async audioBufferForDigest(_context: BaseAudioContext, digest: string) {
      declared.push(digest); return decoded;
    },
  });
  const context = { sampleRate: 4, createBuffer: (_channels: number, length: number) => pcmBuffer([
    new Float32Array(length), new Float32Array(length),
  ]) } as unknown as BaseAudioContext;
  for (let run = 0; run < 2; run++) {
    const output = await player.renderCompiledAudioBuffer(context, 0, 2);
    expect([...output.getChannelData(0)]).toEqual([...new Float32Array([0.05, 0.1])]);
    expect([...output.getChannelData(1)]).toEqual([...new Float32Array([-0.1, -0.15])]);
  }
  expect(declared).toEqual([`sha256:${LEFT_DIGEST}`, `sha256:${LEFT_DIGEST}`]);
  expect(registered).toEqual([`sha256:${LEFT_DIGEST}`]);
  engine.mix_audio_pcm = () => new Float32Array(1);
  await expect(player.renderCompiledAudioBuffer(context, 0, 2)).rejects.toThrow("sample range");
});

test("audio decode finishing after package replacement never writes into the retired engine", async () => {
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  Object.assign(player, {
    renderId: RENDER_ID,
    engine: {
      audio_sources_json: () => JSON.stringify([{ sourceIndex: 0, digest: `sha256:${LEFT_DIGEST}`, sourceChannels: 2 }]),
      has_audio_pcm: () => { throw new Error("retired engine touched"); },
    },
    requireCompiledAudioProgram: () => ({ sampleRate: 4, sampleCount: 1 }),
    async audioBufferForDigest() { player.engine = {}; return pcmBuffer([[0.1], [-0.2]]); },
  });
  const context = { sampleRate: 4, createBuffer: () => pcmBuffer([[0], [0]]) } as unknown as BaseAudioContext;
  await expect(player.renderCompiledAudioBuffer(context, 0, 1)).rejects.toThrow("superseded");
});

test("failed render-package staging preserves the last-good runtime", async () => {
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  let engineFreed = 0;
  let plannerClosed = 0;
  let executorDisposed = 0;
  let resumed = 0;
  const oldEngine = {
    frame_at_seconds: (renderId: string) => {
      expect(renderId).toBe(RENDER_ID);
      return 7n;
    },
    free: () => { engineFreed += 1; },
  };
  const oldPlanner = { close: () => { plannerClosed += 1; } };
  const activeExecutor = { dispose: () => { executorDisposed += 1; } };
  Object.assign(player, {
    closed: false,
    playing: true,
    renderInFlight: null,
    renderId: RENDER_ID,
    engine: oldEngine,
    planner: oldPlanner,
    compositor: activeExecutor,
    fixedPackageManifestJson: "old-package-manifest",
    timelineJson: "old-timeline",
    timeline: { document: {} },
    resourceManifestJson: "old-resource-manifest",
    resourceManifest: { entries: {} },
    verifiedBindingBundleJson: "old-bundle",
    assets: [],
    pause() { this.playing = false; },
    async play() { resumed += 1; this.playing = true; },
    async stageRenderPackage() { throw new Error("staged render package rejected"); },
  });

  await expect(player.replaceRenderPackage({ timelineJson: "new-timeline" }))
    .rejects.toThrow("staged render package rejected");

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
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  const disposed: string[] = [];
  let committed = false;
  Object.assign(player, {
    closed: false, playing: false, renderInFlight: null, timelineJson: "last-good",
    pause() {},
    async stageRenderPackage() {
      return {
        compositor: { dispose: () => disposed.push("compositor") },
        planner: { close: () => disposed.push("planner") },
        engine: { free: () => disposed.push("engine") },
      };
    },
    commitStagedRenderPackage() { committed = true; },
  });
  expect(await player.replaceRenderPackage({ timelineJson: "obsolete", isCurrent: () => false })).toBeUndefined();
  expect(committed).toBe(false);
  expect(player.timelineJson).toBe("last-good");
  expect(disposed).toEqual(["compositor", "planner", "engine"]);
});

test("video source requests decode canonical rational time strings", async () => {
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  const asset = { type: "video" };
  const image = { delete() {} };
  const times: number[] = [];
  Object.assign(player, {
    assetByDigest: new Map([[SCENE_DIGEST.slice(7), asset]]),
    async videoImage(actual: unknown, time: number) { expect(actual).toBe(asset); times.push(time); return { image, texture: false, dispose() {} }; },
  });
  const request = { key: { content: SCENE_DIGEST, interpretation: { kind: "video" } },
    expected: { kind: "visualFrame", extent: { width: 64, height: 36 }, pixel_layout: "rgba8" }, sample: { kind: "sourceTime", time: "1001/30000" } };
  const result = await player.fulfillRequestUncached(request, false);
  expect(result.object.image).toBe(image);
  expect(times).toEqual([1001 / 30000]);
  for (const time of ["", "0/0", { numerator: 1, denominator: 30 }, 0.5]) {
    await expect(player.fulfillRequestUncached({ ...request, sample: { ...request.sample, time } }, false)).rejects.toThrow("RationalTime");
  }
});
