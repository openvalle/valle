import { describe, expect, test } from "bun:test";

import {
  VallePlayerController,
  type VallePlayerOptions,
  type ValleRenderResult,
} from "./controller.ts";
import type { BrowserValleWebPlayer } from "@valle/player-core";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function renderResult(label: string): ValleRenderResult {
  return {
    implementation: label,
    runtimeFlavor: "product-cpu",
    executionProfile: {
      surfaceBackend: "canvaskit-cpu",
      glyphCoverage: {
        rasterizer: "freetype",
        library: "renderer-embedded",
        hinting: "none",
        edging: "antialias",
        subpixelPositioning: false,
      },
      canvasKitVersion: "0.42.0",
    },
    fontsRegistered: 0,
    png: null,
    frame: { renderId: "test", index: 0, planBytes: 0, bindingBytes: 0 },
    stats: {
      videoFrames: 0,
      gpuVideoFrames: 0,
      lottieFrames: 0,
      passes: 0,
      programs: 0,
      physicalSurfaces: 0,
      maximumLiveImages: 0,
      surfaceAllocations: 0,
      surfaceReuses: 0,
    },
  } as ValleRenderResult;
}

function fakePlayer() {
  const replacements: string[] = [];
  let currentTime = 0;
  let closed = 0;
  const runtime = {
    playing: false,
    stats: { degradations: [] },
    onTimeUpdate: undefined as ((timeS: number) => void) | undefined,
    durationS: () => 10,
    currentTime: () => currentTime,
    play: async () => {
      runtime.playing = true;
      runtime.onTimeUpdate?.(currentTime);
    },
    pause: () => { runtime.playing = false; },
    seek: async (timeS: number) => {
      currentTime = timeS;
      runtime.onTimeUpdate?.(currentTime);
      return renderResult(`seek-${timeS}`);
    },
    scrub: async (timeS: number) => {
      currentTime = timeS;
      runtime.onTimeUpdate?.(currentTime);
      return renderResult(`capture-${timeS}`);
    },
    replaceRenderPackage: async (next: { timelineJson: string }) => {
      replacements.push(next.timelineJson);
      return renderResult(next.timelineJson);
    },
    close: async () => { closed += 1; },
  };
  return {
    runtime: runtime as unknown as BrowserValleWebPlayer,
    replacements,
    closed: () => closed,
  };
}

const options = {
  fixedPackageManifestJson: "{}",
  timelineJson: "{}",
  resourceManifestJson: "{}",
  verifiedBindingBundleJson: "{}",
  runtimeAssets: {
    schemaVersion: 1,
    engine: { glue: "engine.js", wasm: "engine.wasm" },
    canvasKit: {
      full: { glue: "full.js", wasm: "full.wasm" },
    },
  },
} as VallePlayerOptions;

function replacement(timelineJson: string) {
  return {
    fixedPackageManifestJson: options.fixedPackageManifestJson,
    timelineJson,
    resourceManifestJson: options.resourceManifestJson,
    verifiedBindingBundleJson: options.verifiedBindingBundleJson,
  };
}

describe("VallePlayerController", () => {
  test("moves new → loading → ready and publishes transport/render events", async () => {
    const fake = fakePlayer();
    const loading = deferred<BrowserValleWebPlayer>();
    const controller = new VallePlayerController(options, () => loading.promise);
    const states: string[] = [];
    const events: string[] = [];
    controller.addEventListener("statechange", (event) => states.push(event.detail.state));
    controller.addEventListener("play", (event) => events.push(`play:${event.detail.playing}`));
    controller.addEventListener("time", (event) => events.push(`time:${event.detail.timeS}`));
    controller.addEventListener("render", (event) => events.push(`render:${event.detail.timeS}`));

    const ready = controller.load();
    expect(controller.state).toBe("loading");
    loading.resolve(fake.runtime);
    await ready;
    expect(controller.state).toBe("ready");
    await controller.play();
    controller.pause();
    await controller.seek(3);

    expect(states).toEqual(["loading", "ready"]);
    expect(events).toContain("play:true");
    expect(events).toContain("play:false");
    expect(events).toContain("time:3");
    expect(events).toContain("render:3");
  });

  test("coalesces queued timeline replacements so the newest generation wins", async () => {
    const fake = fakePlayer();
    const controller = new VallePlayerController(options, async () => fake.runtime);
    await controller.load();

    const old = controller.replaceRenderPackage(replacement("old"));
    const latest = controller.replaceRenderPackage(replacement("latest"));

    expect(await old).toBeUndefined();
    expect((await latest)?.frame).toEqual({
      renderId: "test",
      index: 0,
      planBytes: 0,
      bindingBytes: 0,
    });
    expect(fake.replacements).toEqual(["latest"]);
  });

  test("dispose supersedes an in-flight load and is idempotent", async () => {
    const fake = fakePlayer();
    const loading = deferred<BrowserValleWebPlayer>();
    const controller = new VallePlayerController(options, () => loading.promise);
    const ready = controller.load();
    const firstDispose = controller.dispose();
    const secondDispose = controller.dispose();
    expect(firstDispose).toBe(secondDispose);
    expect(controller.state).toBe("disposed");

    loading.resolve(fake.runtime);
    await expect(ready).rejects.toMatchObject({ name: "AbortError" });
    await firstDispose;
    expect(fake.closed()).toBe(1);
    expect(() => controller.pause()).toThrow("disposed player");
  });
});
