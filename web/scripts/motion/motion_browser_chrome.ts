/** Run the existing /bench page in a fresh, headless Google Chrome profile.
 * The page writes its own measurement to the supplied server result path. This
 * runner only waits for that result and verifies that Chrome used the real GPU.
 *
 * bun web/scripts/motion/motion_browser_chrome.ts http://127.0.0.1:PORT/bench /absolute/result.json
 */
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const url = Bun.argv[2];
const resultPath = Bun.argv[3] && resolve(Bun.argv[3]);
if (!url || !resultPath) throw new Error("expected benchmark URL and new result path");
if (await Bun.file(resultPath).exists()) throw new Error(`result already exists: ${resultPath}`);
const chrome = Bun.env.VALLE_BENCH_CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const profile = await mkdtemp(join(tmpdir(), "valle-b01-chrome-"));
const process = Bun.spawn([
  chrome,
  "--headless=new",
  "--enable-gpu",
  "--use-angle=metal",
  "--disable-software-rasterizer",
  "--enable-logging=stderr",
  "--no-first-run",
  "--no-default-browser-check",
  `--user-data-dir=${profile}`,
  url,
], { stdout: "ignore", stderr: "inherit" });

try {
  const deadline = Date.now() + 180_000;
  let result: Record<string, unknown> | undefined;
  while (Date.now() < deadline) {
    if (await Bun.file(resultPath).exists()) {
      try {
        result = await Bun.file(resultPath).json();
        break;
      } catch {
        // The benchmark server may still be completing its write.
      }
    }
    if (process.exitCode !== null) throw new Error(`Chrome exited with code ${process.exitCode}`);
    await Bun.sleep(250);
  }
  if (!result) throw new Error("Chrome benchmark did not produce a result within 180 seconds");
  if (result.error) throw new Error(String(result.error));
  const renderer = (result.gpu as { renderer?: string } | undefined)?.renderer ?? "";
  if (result.runtimeFlavor !== "product-gpu" || result.surfaceBackend !== "canvaskit-gpu"
      || !renderer.includes("Apple") || !renderer.includes("Metal")) {
    throw new Error(`benchmark did not use the Apple GPU: ${JSON.stringify({
      runtimeFlavor: result.runtimeFlavor, surfaceBackend: result.surfaceBackend, renderer,
    })}`);
  }
  console.log(JSON.stringify({
    resultPath, rows: result.rows, frames: result.frames, browser: result.browser,
    renderer, wallMs: result.wallMs, stages: result.stages,
    playback: result.playback,
  }));
} finally {
  process.kill();
  await process.exited;
  await rm(profile, { recursive: true, force: true });
}
