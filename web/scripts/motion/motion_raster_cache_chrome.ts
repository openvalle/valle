/** Verify RasterProgram caching and host animation through a held source on Chrome’s CanvasKit GPU backend.
 * Run after `bun run build:runtime` from `web`:
 *   bun web/scripts/motion/motion_raster_cache_chrome.ts
 */
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, sep } from "node:path";

const root = resolve(import.meta.dir, "../../..");
const dist = join(root, "web/dist");
const temporary = await mkdtemp(join(tmpdir(), "valle-cache-chrome-"));
const resultPath = join(temporary, "result.json");
const source = join(temporary, "scene.motion.tsx");
const timeline = join(temporary, "timeline.json");
await writeFile(source, `
  export const composition = { width: 48, height: 48, fps: 30, duration: 1 };
  export default function CacheScene(ctx) {
    return <Scene style={{ width: 48, height: 48, backgroundColor: "#000" }}>
      <View style={{ position: "absolute", left: ctx.host.progress * 24,
        top: ctx.seconds * 8, width: 16, height: 16, backgroundColor: "#0f0" }} />
    </Scene>;
  }
`);
await writeFile(timeline, JSON.stringify({
  canvas: { width: 48, height: 48, fps: 30 },
  resources: { scene: "scene.motion.tsx" },
  tracks: { visual: [{ clips: [{ kind: "motion", component: "scene", start: 0, duration: 2, end: "hold" }] }] },
}));
const studio = Bun.spawn([
  process.env.VALLE_TEST_CLI ?? join(root, "target/debug/valle"), "--json", "timeline", "studio",
  timeline, "--port", "0", "--web-assets-dir", dist,
], { cwd: temporary, stdout: "pipe", stderr: "pipe" });
let chrome: ReturnType<typeof Bun.spawn> | null = null;
let server: ReturnType<typeof Bun.serve> | null = null;
try {
  const reader = studio.stdout.getReader();
  let line = "";
  try {
    while (!line.includes("\n")) {
      const part = await reader.read();
      if (part.done) throw new Error(await new Response(studio.stderr).text());
      line += new TextDecoder().decode(part.value);
    }
  } finally { reader.releaseLock(); }
  const upstream = new URL(JSON.parse(line.split("\n")[0]!).url).origin;
  const client = `
    const canvas = document.querySelector("canvas");
    try {
      const { createBrowserValleWebPlayer } = await import("/packages/engine/index.mjs");
      const config = await (await fetch("/config.json")).json();
      if (!config.fixedPackageManifestJson) throw new Error(JSON.stringify(config.diagnostics ?? config));
      const player = await createBrowserValleWebPlayer({
        fixedPackageManifestJson: config.fixedPackageManifestJson,
        timelineJson: config.timelineJson,
        resourceManifestJson: config.resourceManifestJson,
        verifiedBindingBundleJson: config.verifiedBindingBundleJson,
        runtimeAssets: config.runtimeAssets,
        runtimeBaseUrl: location.href,
        canvas, gpu: true, audioContext: null,
      });
      try {
        const first = await player.renderFrame(0);
        const firstPixels = canvas.toDataURL();
        const repeat = await player.renderFrame(0);
        const repeatPixels = canvas.toDataURL();
        const moved = await player.renderFrame(30);
        const movedPixels = canvas.toDataURL();
        await player.renderFrame(59);
        const lastPixels = canvas.toDataURL();
        const revisit = await player.renderFrame(0);
        const revisitPixels = canvas.toDataURL();
        const gl = document.createElement("canvas").getContext("webgl2");
        const ext = gl.getExtension("WEBGL_debug_renderer_info");
        const renderer = ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
        const result = {
          renderer, runtimeFlavor: revisit.runtimeFlavor,
          backend: revisit.executionProfile.surfaceBackend,
          first: first.stats, repeat: repeat.stats, moved: moved.stats,
          revisit: revisit.stats,
          sameRepeat: firstPixels === repeatPixels,
          changedFrame: firstPixels !== movedPixels,
          hostContinuesAfterHold: movedPixels !== lastPixels,
          sameRevisit: firstPixels === revisitPixels,
        };
        await fetch("/result", { method: "POST", body: JSON.stringify(result) });
      } finally { player.close(); }
    } catch (error) {
      await fetch("/result", { method: "POST", body: JSON.stringify({ error: String(error.stack || error) }) });
    }
  `;
  server = Bun.serve({ hostname: "127.0.0.1", port: 0, async fetch(request) {
    const url = new URL(request.url);
    if (process.env.VALLE_CACHE_BROWSER_DEBUG) console.error(request.method, url.pathname);
    if (url.pathname === "/") return new Response('<canvas width="48" height="48"></canvas><script type="module" src="/client.js"></script>', {
      headers: { "Content-Type": "text/html" },
    });
    if (url.pathname === "/client.js") return new Response(client, { headers: { "Content-Type": "text/javascript" } });
    if (url.pathname === "/result" && request.method === "POST") {
      await writeFile(resultPath, await request.text());
      return new Response("ok");
    }
    if (url.pathname === "/config.json") return fetch(`${upstream}/config.json`);
    const path = resolve(dist, `.${url.pathname}`);
    if (!path.startsWith(`${dist}${sep}`)) return new Response("Not found", { status: 404 });
    const file = Bun.file(path);
    return await file.exists() ? new Response(file) : new Response("Not found", { status: 404 });
  } });
  const profile = join(temporary, "chrome-profile");
  chrome = Bun.spawn([
    process.env.VALLE_TEST_CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "--headless=new", "--enable-gpu", "--use-angle=metal", "--disable-software-rasterizer",
    "--no-first-run", "--no-default-browser-check", `--user-data-dir=${profile}`, String(server.url),
  ], { stdout: "ignore", stderr: "inherit" });
  if (process.env.VALLE_CACHE_BROWSER_DEBUG) console.error("Chrome URL", String(server.url), "pid", chrome.pid);
  let result: Record<string, any> | null = null;
  const deadline = Date.now() + 45_000;
  while (Date.now() < deadline) {
    if (await Bun.file(resultPath).exists()) {
      result = await Bun.file(resultPath).json();
      break;
    }
    if (chrome.exitCode !== null) throw new Error(`Chrome exited with code ${chrome.exitCode}`);
    await Bun.sleep(200);
  }
  if (!result) throw new Error("Chrome did not report within 45 seconds");
  if (result.error) throw new Error(result.error);
  const summary = {
    renderer: result.renderer,
    runtimeFlavor: result.runtimeFlavor,
    backend: result.backend,
    first: { hits: result.first.rasterLayerCacheHits, misses: result.first.rasterLayerCacheMisses },
    repeat: { hits: result.repeat.rasterLayerCacheHits, misses: result.repeat.rasterLayerCacheMisses },
    moved: { hits: result.moved.rasterLayerCacheHits, misses: result.moved.rasterLayerCacheMisses },
    revisit: { hits: result.revisit.rasterLayerCacheHits, misses: result.revisit.rasterLayerCacheMisses },
    sameRepeat: result.sameRepeat,
    changedFrame: result.changedFrame,
    hostContinuesAfterHold: result.hostContinuesAfterHold,
    sameRevisit: result.sameRevisit,
  };
  if (!summary.renderer.includes("Apple") || !summary.renderer.includes("Metal")
    || summary.runtimeFlavor !== "product-gpu" || summary.backend !== "canvaskit-gpu"
    || !(summary.first.misses >= 1) || !(summary.repeat.hits >= 1)
    || !(summary.revisit.hits >= 1)
    || !summary.sameRepeat || !summary.changedFrame || !summary.sameRevisit || !summary.hostContinuesAfterHold) {
    throw new Error(`GPU raster cache smoke failed: ${JSON.stringify(summary)}`);
  }
  console.log(JSON.stringify(summary));
} finally {
  chrome?.kill();
  if (chrome) await chrome.exited;
  server?.stop();
  studio.kill();
  await studio.exited;
  await rm(temporary, { recursive: true, force: true });
}
