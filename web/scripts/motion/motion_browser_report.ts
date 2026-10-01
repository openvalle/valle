/** Summarize repeated B01 Chrome GPU samples without assuming a frame-rate gate. */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { median, option, ROOT, sha256 } from "./common.ts";

type Result = {
  error?: unknown;
  runtimeFlavor: string;
  surfaceBackend: string;
  gpu: { renderer: string };
  frames: number;
  canvas: string;
  browser: string;
  rows: number;
  stages: Record<string, number>;
  wallMs: { p50: number; p95: number };
  playback: { presented: number; uniqueFrames: number; intervalP95Ms: number; lastFrame: number };
};

function main() {
  const args = Bun.argv.slice(2);
  const output = resolve(option(args, "--output"));
  const outputIndex = args.indexOf("--output");
  const paths = args.filter((_, index) => index !== outputIndex && index !== outputIndex + 1);
  if (paths.length < 2) throw new Error("provide repeated results for at least two instance counts");
  const rows = paths.map(path => ({ path, result: JSON.parse(readFileSync(path, "utf8")) as Result }));
  for (const { path, result } of rows) {
    if (result.error || result.runtimeFlavor !== "product-gpu"
      || result.surfaceBackend !== "canvaskit-gpu"
      || !result.gpu?.renderer.includes("Apple") || !result.gpu.renderer.includes("Metal")
      || result.frames !== 60 || result.canvas !== "640x360") {
      throw new Error(`invalid real-GPU 60-frame sample: ${path}`);
    }
  }
  const browsers = new Set(rows.map(row => row.result.browser));
  const renderers = new Set(rows.map(row => row.result.gpu.renderer));
  if (browsers.size !== 1 || renderers.size !== 1) {
    throw new Error("all samples must use the same Chrome build and GPU renderer");
  }
  const grouped = new Map<number, typeof rows>();
  for (const row of rows) grouped.set(row.result.rows, [...(grouped.get(row.result.rows) ?? []), row]);
  if (grouped.size < 2) throw new Error("samples must include at least two instance counts");
  const summary: Record<string, unknown> = {};
  for (const [count, samples] of [...grouped].sort(([a], [b]) => a - b)) {
    const results = samples.map(sample => sample.result);
    const stages = Object.keys(results[0]!.stages);
    summary[String(count)] = {
      runs: samples.length,
      files: samples.map(sample => sample.path),
      wallP50Ms: median(results.map(result => result.wallMs.p50)),
      wallP95Ms: median(results.map(result => result.wallMs.p95)),
      stageMeanMs: Object.fromEntries(stages.map(stage => [stage,
        median(results.map(result => result.stages[stage]!))])),
      playbackPresented: results.map(result => result.playback.presented),
      playbackUniqueFrames: results.map(result => result.playback.uniqueFrames),
      playbackIntervalP95Ms: median(results.map(result => result.playback.intervalP95Ms)),
      playbackLastFrame: results.map(result => result.playback.lastFrame),
    };
  }
  const report = {
    browser: [...browsers][0], gpuRenderer: [...renderers][0],
    runtimeFlavor: "product-gpu", surfaceBackend: "canvaskit-gpu",
    runtimeBundleSha256: sha256(join(ROOT, "web/dist/packages/engine/index.mjs")),
    benchmarkClientSha256: sha256(join(import.meta.dir, "motion_browser_client.ts")),
    chromeRunnerSha256: sha256(join(import.meta.dir, "motion_browser_chrome.ts")),
    fixtureSha256: sha256(join(ROOT, "crates/valle-compiler/tests/fixtures/motion/composition/grid-instances.motion.tsx")),
    canvas: "640x360", framesPerRun: 60, summary,
  };
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify({ output, summary }));
}

if (import.meta.main) main();
