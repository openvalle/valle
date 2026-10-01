import type { BrowserValleWebPlayerOptions, ProductRenderResult } from "../../packages/engine/src/index.ts";

// Load the built runtime in the browser while checking against the source API.
const runtimeUrl = "/packages/engine/index.mjs";
const { createBrowserValleWebPlayer } = await import(runtimeUrl) as
  Pick<typeof import("../../packages/engine/src/index.ts"), "createBrowserValleWebPlayer">;

type BenchmarkConfig = Pick<BrowserValleWebPlayerOptions,
  "fixedPackageManifestJson" | "timelineJson" | "resourceManifestJson" |
  "verifiedBindingBundleJson" | "runtimeAssets"> & {
  status: string;
  diagnostics?: unknown;
  durationFrames: number;
  instanceRows: number;
};

const status = document.querySelector<HTMLElement>("#status")!;
const report = document.querySelector<HTMLElement>("#report")!;
const canvas = document.querySelector<HTMLCanvasElement>("#preview")!;
const requestedFrames = Number(new URLSearchParams(location.search).get("frames") ?? 60);
const frames = Number.isInteger(requestedFrames) && requestedFrames >= 1 && requestedFrames <= 300
  ? requestedFrames : 60;

function percentile(values: number[], fraction: number) {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(fraction * (sorted.length - 1)))];
}

function gpuIdentity() {
  const probe = document.createElement("canvas");
  const gl = probe.getContext("webgl2") || probe.getContext("webgl");
  if (!gl) return { webgl: false };
  const extension = gl.getExtension("WEBGL_debug_renderer_info");
  return {
    webgl: true,
    vendor: extension ? gl.getParameter(extension.UNMASKED_VENDOR_WEBGL) : gl.getParameter(gl.VENDOR),
    renderer: extension ? gl.getParameter(extension.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER),
  };
}

async function run() {
  const config = await (await fetch("/config.json", { cache: "no-store" })).json() as BenchmarkConfig;
  if (config.status !== "ok") throw new Error(`Motion preparation failed: ${JSON.stringify(config.diagnostics)}`);
  status.textContent = "Initializing real browser player…";
  const player = await createBrowserValleWebPlayer({
    fixedPackageManifestJson: config.fixedPackageManifestJson,
    timelineJson: config.timelineJson,
    resourceManifestJson: config.resourceManifestJson,
    verifiedBindingBundleJson: config.verifiedBindingBundleJson,
    runtimeAssets: config.runtimeAssets,
    runtimeBaseUrl: location.href,
    canvas,
    gpu: true,
    audioContext: null,
  });
  try {
    for (let i = 0; i < 5; i++) await player.renderFrame(i);
    const before = { ...player.stats };
    const elapsed: number[] = [];
    const presentedAt: number[] = [];
    let lastResult: ProductRenderResult | undefined;
    status.textContent = `Measuring ${frames} frames…`;
    for (let i = 0; i < frames; i++) {
      await new Promise<number>(requestAnimationFrame);
      const started = performance.now();
      lastResult = await player.renderFrame(i % config.durationFrames);
      presentedAt.push(performance.now());
      elapsed.push(performance.now() - started);
      if (i % 10 === 9) status.textContent = `Measured ${i + 1}/${frames} frames…`;
    }
    if (!lastResult) throw new Error("No benchmark frames were rendered");
    const after = player.stats;
    const fields = [
      "perfFrameMs", "perfEvaluatePrepareMs", "perfRequestInspectMs", "perfLowerMs",
      "perfBindPacketsMs", "perfFramePlannerWorkerMs", "perfExecuteMs",
      "perfExecutorPacketAdmissionMs", "perfExecutorProgramAdmissionMs",
      "perfExecutorScheduleAdmissionMs", "perfExecutorRenderMs", "perfPresentMs",
    ] as const;
    const stages = Object.fromEntries(fields.map(key => [key, (after[key] - before[key]) / frames]));
    const intervals = presentedAt.slice(1).map((value, index) => value - presentedAt[index]);
    const measured = {
      rows: config.instanceRows,
      frames,
      canvas: `${canvas.width}x${canvas.height}`,
      browser: navigator.userAgent,
      gpu: gpuIdentity(),
      runtimeFlavor: lastResult.runtimeFlavor,
      surfaceBackend: lastResult.executionProfile.surfaceBackend,
      wallMs: { p50: percentile(elapsed, 0.5), p95: percentile(elapsed, 0.95), max: Math.max(...elapsed) },
      presentationMs: { p50: percentile(intervals, 0.5), p95: percentile(intervals, 0.95), max: Math.max(...intervals) },
      presentedFps: (frames - 1) * 1000 / (presentedAt.at(-1)! - presentedAt[0]),
      over33ms: elapsed.filter(value => value > 33.333).length,
      stages,
      lastFrame: { ...lastResult.frame, renderId: "<omitted>" },
      lastStats: lastResult.stats,
    };
    await player.seek(0);
    const played: Array<{ frame: number; atMs: number }> = [];
    player.onFramePresented = (frame, atMs) => played.push({ frame, atMs });
    const playbackStarted = performance.now();
    await player.play();
    while (player.playing && performance.now() - playbackStarted < 10000) {
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    const playbackEnded = performance.now();
    player.pause();
    const playbackIntervals = played.slice(1).map((item, index) => item.atMs - played[index].atMs);
    const playback = {
      presented: played.length,
      totalFrames: config.durationFrames,
      elapsedMs: playbackEnded - playbackStarted,
      uniqueFrames: new Set(played.map(item => item.frame)).size,
      firstFrame: played[0]?.frame ?? null,
      lastFrame: played.at(-1)?.frame ?? null,
      intervalP50Ms: playbackIntervals.length ? percentile(playbackIntervals, 0.5) : null,
      intervalP95Ms: playbackIntervals.length ? percentile(playbackIntervals, 0.95) : null,
      frames: played.map(item => item.frame),
    };
    const result = { ...measured, playback };
    status.textContent = "Measurement complete";
    report.textContent = JSON.stringify(result, null, 2);
    await fetch("/bench/result", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(result) });
  } finally {
    player.close();
  }
}

run().catch(async error => {
  status.textContent = `Benchmark failed: ${error.stack || error}`;
  await fetch("/bench/result", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ error: String(error.stack || error) }) });
});
