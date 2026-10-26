// End-to-end Scene3D acceptance, including the reference/export worker configuration.
import {
  createBrowserValleWebPlayer,
  type BrowserValleWebPlayer,
  type BrowserValleWebPlayerOptions,
} from "../../../packages/engine/src/runtime/product-controller.ts";
import { exportBrowserVideo } from "../../../packages/engine/src/export.ts";

async function run() {
  const config = (await (await fetch("/config.json")).json()) as BrowserValleWebPlayerOptions & {
    resourceBytesUrl: string;
    resourceLocators?: BrowserValleWebPlayerOptions["assets"];
  };
  config.assets = config.resourceLocators ?? config.assets;
  config.resourceBytes = new Uint8Array(await (await fetch(config.resourceBytesUrl)).arrayBuffer());
  const players: BrowserValleWebPlayer[] = [];
  const canvases = [document.createElement("canvas"), document.createElement("canvas")];
  document.body.replaceChildren(...canvases);
  const rows = [];
  const creationMs = [];
  const features = new URLSearchParams(location.search).get("features");
  const expectedPrograms = features === "unlit" ? 2 : features === "assets" ? 6 : 5;
  // The optional 4K comparison keeps a CPU reference player beside the GPU player.
  const memoryBytes =
    JSON.parse(config.timelineJson).document.canvas.width >= 3840 ? 4 * 1024 ** 3 : undefined;
  try {
    for (const [index, gpu] of [false, true].entries()) {
      const started = performance.now();
      players.push(
        await createBrowserValleWebPlayer({
          ...config,
          canvas: canvases[index],
          gpu,
          memoryBytes,
          renderMode: "reference",
          runtimePurpose: "export",
          runtimeBaseUrl: location.href,
        }),
      );
      creationMs.push(performance.now() - started);
    }
    const seen = new Map<number, Uint8ClampedArray>();
    const frames = memoryBytes ? [27] : [27, 0, 15, 27];
    for (const frame of frames) {
      const pixels: Uint8ClampedArray[] = [];
      const timings = [];
      for (let i = 0; i < players.length; i++) {
        const started = performance.now();
        await players[i]!.renderFrame(frame);
        const canvas = canvases[i]!;
        pixels.push(
          canvas
            .getContext("2d", { willReadFrequently: true })!
            .getImageData(0, 0, canvas.width, canvas.height).data,
        );
        timings.push(performance.now() - started);
      }
      const [cpu, gpu] = pixels as [Uint8ClampedArray, Uint8ClampedArray];
      let maximum = 0,
        squared = 0;
      for (let i = 0; i < cpu.length; i++) {
        const difference = Math.abs(cpu[i]! - gpu[i]!);
        maximum = Math.max(maximum, difference);
        squared += difference * difference;
      }
      const psnr = squared === 0 ? 999 : 10 * Math.log10((255 * 255 * cpu.length) / squared);
      if (maximum > 2 || psnr < 50) {
        const differences = [];
        for (let i = 0; i < cpu.length && differences.length < 12; i += 4) {
          if (cpu.subarray(i, i + 4).some((v, j) => Math.abs(v - gpu[i + j]!) > 2))
            differences.push({
              x: (i / 4) % canvases[0]!.width,
              y: Math.floor(i / 4 / canvases[0]!.width),
              cpu: Array.from(cpu.subarray(i, i + 4)),
              gpu: Array.from(gpu.subarray(i, i + 4)),
            });
        }
        for (const [i, name] of ["cpu", "gpu"].entries()) {
          const png = await new Promise<Blob>((resolve, reject) =>
            canvases[i]!.toBlob((blob) =>
              blob ? resolve(blob) : reject(new Error("Scene3D capture failed")),
            ),
          );
          await fetch(`/scene3d-${name}.png`, { method: "POST", body: png });
        }
        throw new Error(
          `Scene3D frame ${frame}: max=${maximum}, PSNR=${psnr}, differences=${JSON.stringify(differences)}`,
        );
      }
      const prior = seen.get(frame);
      if (prior && prior.some((v, i) => v !== gpu[i]))
        throw new Error(`Scene3D seek changed frame ${frame}`);
      seen.set(frame, gpu);
      const diagnostics = players[1]!.scene3dGpuDiagnostics;
      if (diagnostics?.programCompiles !== expectedPrograms)
        throw new Error(
          `Scene3D compiled unused or duplicate programs: ${JSON.stringify(diagnostics)}`,
        );
      rows.push({ frame, maximum, psnr, cpuMs: timings[0], gpuMs: timings[1] });
    }
    const diagnostics = players[1]!.scene3dGpuDiagnostics;
    if (!diagnostics || diagnostics.renders < frames.length || diagnostics.fallbacks !== 0)
      throw new Error(`Scene3D did not rasterize on GPU: ${JSON.stringify(diagnostics)}`);
    if (
      features === "unlit"
        ? diagnostics.atlasUploads !== 0 || diagnostics.shadowRenders !== 0
        : diagnostics.atlasUploads !== 1 || diagnostics.shadowRenders < 2
    )
      throw new Error(
        `Scene3D atlas/shadow path was not exercised: ${JSON.stringify(diagnostics)}`,
      );
    let encoded: Record<string, unknown> | undefined;
    if (new URLSearchParams(location.search).has("encode")) {
      for (const player of players) await player.close();
      const exported = await exportBrowserVideo(
        { ...config, gpu: true, renderMode: "reference", runtimeBaseUrl: location.href },
        { includeAudio: false, videoBitrate: 4_000_000 },
      );
      if (
        !exported.blob?.size ||
        exported.frameCount !== 60 ||
        exported.executionBackends?.join() !== "canvaskit-gpu"
      )
        throw new Error("Scene3D export did not produce 60 GPU frames");
      const video = document.createElement("video"),
        url = URL.createObjectURL(exported.blob);
      try {
        const loaded = new Promise<void>((resolve, reject) => {
          video.onloadeddata = () => resolve();
          video.onerror = () => reject(new Error("Scene3D MP4 cannot be decoded"));
        });
        video.src = url;
        await loaded;
        if (
          video.videoWidth !== exported.width ||
          video.videoHeight !== exported.height ||
          Math.abs(video.duration - 2) > 0.001
        )
          throw new Error("Scene3D MP4 dimensions or duration changed");
        const seeked = new Promise<void>((resolve) => {
          video.onseeked = () => resolve();
        });
        video.currentTime = 0.5;
        await seeked;
        const canvas = canvases[1]!,
          context = canvas.getContext("2d", { willReadFrequently: true })!;
        context.drawImage(video, 0, 0);
        const decoded = context.getImageData(0, 0, canvas.width, canvas.height).data,
          expected = seen.get(15)!;
        let squared = 0;
        for (let i = 0; i < decoded.length; i++) squared += (decoded[i]! - expected[i]!) ** 2;
        const psnr = 10 * Math.log10((255 * 255 * decoded.length) / squared);
        if (psnr < 35) throw new Error(`Scene3D MP4 frame differs from its source: PSNR=${psnr}`);
        encoded = {
          bytes: exported.blob.size,
          frames: exported.frameCount,
          psnr,
          backends: exported.executionBackends,
        };
        const saved = await fetch("/scene3d.mp4", { method: "POST", body: exported.blob });
        if (!saved.ok) throw new Error("Scene3D MP4 artifact was not saved");
      } finally {
        video.removeAttribute("src");
        video.load();
        URL.revokeObjectURL(url);
      }
    }
    return {
      status: "ok",
      stats: { creationMs, rows, scene3dGpu: diagnostics, encoded },
      captures: [],
    };
  } finally {
    for (const player of players) await player.close();
  }
}

const report = await run().catch((error) => ({ status: "error", message: String(error) }));
await fetch("/result", {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify(report),
});
