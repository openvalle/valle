import { expect, test } from "bun:test";
import { BrowserResourceCache } from "./resource-cache.ts";

import {
  BrowserValleWebPlayer,
  mixCommonAudioBlockPcm,
  type AudioBlockWire,
  type AudioEndpointWire,
} from "./product-controller.ts";

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

test("preview uses native audio decoding without requiring identical PCM bytes", async () => {
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  const encoded = Uint8Array.from([1, 2, 3]);
  const decoded = { sampleRate: 48000, numberOfChannels: 2 };
  let decodes = 0;
  Object.assign(player, {
    audioBuffers: new Map(),
    assetByDigest: new Map([[LEFT_DIGEST, { id: "dialogue" }]]),
    async fetchAssetBytes() { return encoded; },
  });
  const context = {
    sampleRate: 48000,
    async decodeAudioData(bytes: ArrayBuffer) {
      decodes++;
      expect(new Uint8Array(bytes)).toEqual(encoded);
      expect(bytes).not.toBe(encoded.buffer); // Native decoding can detach its input.
      return decoded;
    },
  } as unknown as BaseAudioContext;
  expect(await player.audioBufferForDigest(context, `sha256:${LEFT_DIGEST}`, 2)).toBe(decoded);
  expect(await player.audioBufferForDigest(context, `sha256:${LEFT_DIGEST}`, 2)).toBe(decoded);
  expect(decodes).toBe(1);
  await expect(player.audioBufferForDigest(context, `sha256:${LEFT_DIGEST}`, 1)).rejects.toThrow("channels; expected");
  decoded.sampleRate = 44100;
  player.audioBuffers.clear();
  await expect(player.audioBufferForDigest(context, `sha256:${LEFT_DIGEST}`, 2)).rejects.toThrow("44100 Hz");
});

function pcmBuffer(channels: number[][]): AudioBuffer {
  const data = channels.map((channel) => Float32Array.from(channel));
  return {
    length: data[0]!.length,
    numberOfChannels: data.length,
    getChannelData(channel: number) {
      return data[channel]!;
    },
  } as AudioBuffer;
}

function endpoint(
  sourceIndex: number,
  sourceSampleIndex: number,
  leftGain: number,
  rightGain: number,
): AudioEndpointWire {
  const left = sourceIndex === 0;
  return {
    sourceIndex,
    sourceSampleIndex,
    mappedTime: { type: "exact", time: `${sourceSampleIndex}/4` },
    digest: `sha256:${left ? LEFT_DIGEST : RIGHT_DIGEST}`,
    handle: left ? 1 : 2,
    decodedPcmDigest: `sha256:${"3".repeat(64)}`,
    sourceChannels: 2,
    crossfadeGain: 1,
    gain: left ? 0.7 : 0.6,
    pan: left ? -0.25 : 0.4,
    leftGain,
    rightGain,
  };
}

function commonAudioGoldenBlock(): AudioBlockWire {
  const left = (sample: number, crossfade = 1) => endpoint(
    0,
    sample,
    0.004200000000000001 * crossfade,
    0.0031500000000000005 * crossfade,
  );
  const right = (sample: number, crossfade = 1) => endpoint(
    1,
    sample,
    0.108 * crossfade,
    0.18 * crossfade,
  );
  const endpoints = [
    [left(0)],
    [left(1)],
    [left(2)],
    [left(3), right(0, 0)],
    [left(4, 0.5), right(0, 0.5)],
    [left(5, 0), right(1)],
    [right(2)],
    [right(3)],
  ];
  return {
    renderId: RENDER_ID,
    sampleRate: 4,
    startSample: 0,
    endSample: endpoints.length,
    samples: endpoints.map((sampleEndpoints, sample) => ({
      renderId: RENDER_ID,
      sample,
      sampleTime: `${sample}/4`,
      tracks: [{ trackOrder: 0, endpoints: sampleEndpoints }],
    })),
  };
}

function sliceBlock(block: AudioBlockWire, start: number, end: number): AudioBlockWire {
  return {
    ...block,
    startSample: start,
    endSample: end,
    samples: block.samples.slice(start, end),
  };
}

function interleavedBits(left: Float32Array, right: Float32Array): number[] {
  const interleaved = new Float32Array(left.length * 2);
  for (let index = 0; index < left.length; index += 1) {
    interleaved[index * 2] = left[index]!;
    interleaved[index * 2 + 1] = right[index]!;
  }
  const view = new DataView(interleaved.buffer);
  return Array.from(
    { length: interleaved.length },
    (_, index) => view.getUint32(index * 4, true),
  );
}

test("Web common-profile mixer matches the Native fixed PCM bits and chunk/seek boundaries", () => {
  const block = commonAudioGoldenBlock();
  const decoded = new Map([
    [LEFT_DIGEST, pcmBuffer([
      [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
      [-0.2, -0.3, -0.4, -0.5, -0.6, -0.7, -0.8, -0.9],
    ])],
    [RIGHT_DIGEST, pcmBuffer([
      [-0.15, -0.25, -0.35, -0.45, -0.55, -0.65, -0.75, -0.85],
      [0.25, 0.35, 0.45, 0.55, 0.65, 0.75, 0.85, 0.95],
    ])],
  ]);
  const whole = mixCommonAudioBlockPcm(block, decoded);
  expect(interleavedBits(whole.left, whole.right)).toEqual([
    0x39dc3372, 0xba252696, 0x3a5c3372, 0xba77b9e1,
    0x3aa52696, 0xbaa52696, 0x3adc3372, 0xbace703b,
    0xbbe703b0, 0x3cb0941c, 0xbcdd2f1b, 0x3d810625,
    0xbd1ad42c, 0x3da5e354, 0xbd4710cb, 0x3dcac083,
  ]);

  const chunkedLeft: number[] = [];
  const chunkedRight: number[] = [];
  for (const [start, end] of [[0, 1], [1, 3], [3, 6], [6, 8]]) {
    const mixed = mixCommonAudioBlockPcm(sliceBlock(block, start!, end!), decoded);
    chunkedLeft.push(...mixed.left);
    chunkedRight.push(...mixed.right);
  }
  expect(chunkedLeft).toEqual([...whole.left]);
  expect(chunkedRight).toEqual([...whole.right]);

  const seek = mixCommonAudioBlockPcm(sliceBlock(block, 2, 8), decoded);
  expect([...seek.left]).toEqual([...whole.left.slice(2)]);
  expect([...seek.right]).toEqual([...whole.right.slice(2)]);
});

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
        return new Uint8Array(4);
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
      ColorType: { RGBA_8888: 1 },
      AlphaType: { Premul: 1 },
      ColorSpace: { SRGB: 1 },
      MakeImage: () => image,
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

test("compiled audio keeps canonical ContentDigest wire until decode admission", async () => {
  const block = sliceBlock(commonAudioGoldenBlock(), 0, 1);
  const declared: string[] = [];
  const output = [new Float32Array(1), new Float32Array(1)];
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  Object.assign(player, {
    renderId: RENDER_ID,
    engine: { audio_block_json: () => JSON.stringify(block) },
    requireCompiledAudioProgram: () => ({ sampleRate: 4, sampleCount: 1 }),
    async audioBufferForDigest(
      _context: BaseAudioContext,
      digest: string,
    ) {
      declared.push(digest);
      return pcmBuffer([[0.1], [-0.2]]);
    },
  });
  const context = {
    sampleRate: 4,
    createBuffer: () => ({ getChannelData: (channel: number) => output[channel]! }),
  } as unknown as BaseAudioContext;

  await player.renderCompiledAudioBuffer(context, 0, 1);
  expect(declared).toEqual([`sha256:${LEFT_DIGEST}`]);
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


test("video and Lottie source requests decode canonical rational time strings", async () => {
  for (const type of ["video", "lottie"]) {
    const player = Object.create(BrowserValleWebPlayer.prototype) as any;
    const asset = { type };
    const image = { delete() {} };
    const times: number[] = [];
    Object.assign(player, {
      assetByDigest: new Map([[SCENE_DIGEST.slice(7), asset]]),
      async videoImage(actual: unknown, time: number) { expect(actual).toBe(asset); times.push(time); return { image, texture: false, dispose() {} }; },
      async lottieImage(actual: unknown, time: number) { expect(actual).toBe(asset); times.push(time); return image; },
    });
    const request = { key: { content: SCENE_DIGEST, interpretation: { kind: type } },
      expected: { kind: "visualFrame", extent: { width: 64, height: 36 }, pixel_layout: "rgba8" }, sample: { kind: "sourceTime", time: "1001/30000" } };
    const result = await player.fulfillRequestUncached(request, false);
    expect(result.object.image).toBe(image);
    expect(times).toEqual([1001 / 30000]);
    for (const time of ["", "0/0", { numerator: 1, denominator: 30 }, 0.5]) {
      await expect(player.fulfillRequestUncached({ ...request, sample: { ...request.sample, time } }, false)).rejects.toThrow("RationalTime");
    }
  }
});
