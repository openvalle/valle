import { expect, test } from "bun:test";

import {
  BrowserValleWebPlayer,
  commonAudioPcmDigestHex,
  mixCommonAudioBlockPcm,
  type AudioBlockWire,
  type AudioEndpointWire,
} from "./product-controller.ts";

const RENDER_ID = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const SCENE_DIGEST = `sha256:${"4".repeat(64)}`;
const TOPOLOGY_DIGEST = `sha256:${"5".repeat(64)}`;

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

test("common audio decoded PCM digest matches the cross-platform byte domain", async () => {
  const samples = Float32Array.from([0.1, 0.2]);
  const buffer = {
    sampleRate: 4,
    numberOfChannels: 1,
    length: 2,
    getChannelData(channel: number) {
      expect(channel).toBe(0);
      return samples;
    },
  } as AudioBuffer;
  expect(await commonAudioPcmDigestHex(buffer)).toBe(
    "cfe5b3919c9c55851665cf86e0cc1c9409b64e94f9854e6ea8729ad2c9499960",
  );
});

const LEFT_DIGEST = "1111111111111111111111111111111111111111111111111111111111111111";
const RIGHT_DIGEST = "2222222222222222222222222222222222222222222222222222222222222222";

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
      interpretation: { kind: "scene3d", topologyDigest: TOPOLOGY_DIGEST },
    },
    expected: { kind: "scene3d" },
    payload: { kind: "scene3dFrame", canonicalRequest: [1, 2, 3] },
  }, false);

  expect(calls).toEqual([[SCENE_DIGEST, TOPOLOGY_DIGEST]]);
  expect(produced.object.image).toBe(image);
});

test("Scene3D texture admission keeps the canonical ContentDigest wire", async () => {
  const admittedDigests: string[] = [];
  const encoded = Uint8Array.from([1, 2, 3]);
  const pixels = Uint8Array.from([0, 0, 0, 0]);
  const player = Object.create(BrowserValleWebPlayer.prototype) as any;
  Object.assign(player, {
    scene3dResourceTasks: new Map(),
    assetByDigest: new Map([[SCENE_DIGEST.slice("sha256:".length), { id: "texture" }]]),
    engine: {
      register_scene3d_texture(digest: string) {
        admittedDigests.push(digest);
      },
    },
    async fetchAssetBytes() { return encoded; },
    async staticImage() {
      return {
        width: () => 1,
        height: () => 1,
        readPixels: () => pixels,
        delete() {},
      };
    },
    CanvasKit: {
      ColorType: { RGBA_8888: 1 },
      AlphaType: { Premul: 1 },
      ColorSpace: { SRGB: 1 },
    },
  });

  await player.ensureScene3dResource("texture", SCENE_DIGEST);
  expect(admittedDigests).toEqual([SCENE_DIGEST]);
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
