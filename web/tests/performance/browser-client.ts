import type {
  BrowserValleWebPlayerOptions,
  ProductRenderResult,
} from "../../packages/engine/src/index.ts";
const runtimeUrl = "/packages/engine/index.mjs";
const { BrowserValleWebPlayer, RenderTrace } = (await import(
  runtimeUrl
)) as typeof import("../../packages/engine/src/index.ts");
const exportUrl = "/packages/engine/export.mjs";
const { exportBrowserVideo } = (await import(
  exportUrl
)) as typeof import("../../packages/engine/src/export.ts");
const job = await (await fetch("/job.json")).json();
const trace = job.trace ? new RenderTrace() : undefined;
const canvas = document.createElement("canvas");
document.body.append(canvas);
const input = {
  ...job.input,
  canvas,
  gpu: job.gpu,
  audioContext: null,
  trace,
  runtimeBaseUrl: location.href,
} as BrowserValleWebPlayerOptions;
const post = (name: string, bytes: BodyInit) =>
  fetch(`/artifact/${name}`, { method: "POST", body: bytes });

function identity() {
  const probe = document.createElement("canvas");
  const gl = probe.getContext("webgl2") ?? probe.getContext("webgl");
  if (!gl) return { renderer: "unavailable" };
  const ext = gl.getExtension("WEBGL_debug_renderer_info");
  const result = {
    renderer: String(gl.getParameter(ext ? ext.UNMASKED_RENDERER_WEBGL : gl.RENDERER)),
    vendor: String(gl.getParameter(ext ? ext.UNMASKED_VENDOR_WEBGL : gl.VENDOR)),
  };
  gl.getExtension("WEBGL_lose_context")?.loseContext();
  return result;
}

function assertPath(result: ProductRenderResult) {
  const expected = job.gpu ? "canvaskit-gpu" : "canvaskit-cpu";
  if (result.executionProfile.surfaceBackend !== expected)
    throw new Error(`requested ${expected}, got ${result.executionProfile.surfaceBackend}`);
}

async function run() {
  const gpu = identity();
  if (job.gpu && /swiftshader|llvmpipe|software|unavailable/i.test(gpu.renderer))
    throw new Error(`real browser GPU unavailable: ${gpu.renderer}`);
  if (job.cpuOutputF32) {
    const glueUrl = new URL(job.input.runtimeAssets.engine.glue, location.href).href;
    const { ProductEngine } = await import(glueUrl);
    if (typeof ProductEngine.prototype.transform_srgb_preview_f16 === "function")
      throw new Error("F32 diagnostic control did not disable the F16 API");
  }
  if (job.mode === "qa") {
    const outputs: unknown[] = [];
    // A new player provides a cold baseline; repeated/reverse requests exercise warm caches.
    // A second session bypasses the resource cache and resizes away/back to invalidate targets.
    for (const bypass of [false, true]) {
      const player = new BrowserValleWebPlayer({
        ...input,
        ...(bypass ? { resourceCache: { maxEntries: 1, maxBytes: job.resourceCacheBytes } } : {}),
      });
      try {
        await player.init();
        if (bypass) {
          await player.renderFrame(0, { rasterScale: 0.75, transparent: true });
        }
        for (const [sequence, frame] of (job.order as number[]).entries()) {
          const result = await player.renderFrame(frame, {
            rasterScale: job.width / 640,
            transparent: true,
          });
          assertPath(result);
          // Capture the canvas after the requested backend rendered. Never set png:true (CPU).
          const blob = await new Promise<Blob>((resolve, reject) =>
            canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("capture failed")))),
          );
          const name = `${bypass ? "bypass" : "warm"}-${sequence}-${frame}.png`;
          await post(name, blob);
          outputs.push({ name, frame, sequence, bypass, profile: result.executionProfile });
        }
        outputs.push({ session: bypass ? "bypass" : "warm", stats: player.stats });
      } finally {
        await player.close();
      }
    }
    return {
      status: "ok",
      gpu,
      outputs,
      userAgent: navigator.userAgent,
      trace: trace?.snapshot({ backend: job.gpu ? "web-gpu" : "web-cpu" }),
    };
  }
  if (job.mode === "order") {
    const outputs: unknown[] = [];
    // Each order starts from a cold player, so only the access order differs between sessions.
    for (const [order, frames] of Object.entries(job.orders as Record<string, number[]>)) {
      const player = new BrowserValleWebPlayer(input);
      try {
        await player.init();
        for (const frame of frames) {
          assertPath(
            await player.renderFrame(frame, { rasterScale: job.width / 640, transparent: true }),
          );
          const blob = await new Promise<Blob>((resolve, reject) =>
            canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("capture failed")))),
          );
          const name = `${order}-${frame}.png`;
          await post(name, blob);
          outputs.push({ name, frame, order });
        }
        outputs.push({ session: order, stats: player.stats });
      } finally {
        await player.close();
      }
    }
    return { status: "ok", gpu, outputs, userAgent: navigator.userAgent };
  }
  const started = performance.now();
  const result = await exportBrowserVideo(input, {
    width: job.width,
    height: job.height,
    frameRate: 60,
    includeAudio: false,
    videoBitrate: job.bitrate,
  });
  const wallMs = performance.now() - started;
  const expected = job.gpu ? "canvaskit-gpu" : "canvaskit-cpu";
  if (result.executionBackends?.length !== 1 || result.executionBackends[0] !== expected)
    throw new Error(`export backend drift: ${result.executionBackends}`);
  if (
    !result.blob ||
    result.width !== job.width ||
    result.height !== job.height ||
    result.frameCount !== 600
  )
    throw new Error("export schedule/dimensions drift");
  await post("export.mp4", result.blob);
  return {
    status: "ok",
    gpu,
    wallMs,
    result: { ...result, blob: undefined },
    userAgent: navigator.userAgent,
    scope:
      "frozen package -> runtime initialization, rendering, encoding and MP4 finalization; excludes host TSX compilation",
    trace: trace?.snapshot({
      backend: job.gpu ? "web-gpu" : "web-cpu",
      result: { ...result, blob: undefined },
    }),
  };
}

try {
  await fetch("/result", { method: "POST", body: JSON.stringify(await run()) });
} catch (error) {
  await fetch("/result", {
    method: "POST",
    body: JSON.stringify({
      status: "failed",
      error: String(error instanceof Error ? error.stack : error),
    }),
  });
}
