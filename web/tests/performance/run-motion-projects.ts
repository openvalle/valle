/** Native project regressions with explicit source paths, settings and preserved artifacts. */
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { median, option, ROOT, run, sha256 } from "../../scripts/motion/common.ts";

const args = Bun.argv.slice(2);
if (args.includes("--help")) {
  console.log("bun web/tests/performance/run-motion-projects.ts --output NEW_DIR --sources ABSOLUTE_TSX,... [--cli BINARY] [--compare BASELINE/run.json]");
  process.exit(0);
}
const output = path.resolve(option(args, "--output"));
const cli = path.resolve(option(args, "--cli", path.join(ROOT, "target/release/valle")));
const sources = option(args, "--sources").split(",").map(source => path.resolve(source));
const settings = { width: 1920, height: 1080, fps: 30, durationS: 10, workers: 4, bitrate: 20_000_000, repeats: 3 };
const backends = ["raster", "metal"];
const regressionTolerancePercent = 10;
if (process.platform !== "darwin") throw new Error("this project CPU/Metal comparison requires macOS");
if (await Bun.file(path.join(output, "run.json")).exists()) throw new Error("result directory already used");
await mkdir(output, { recursive: true });
const host = { platform: os.platform(), arch: os.arch(), cpu: os.cpus()[0]?.model, cpus: os.cpus().length, memory: os.totalmem() };
const identities = { cli: sha256(cli), gitHead: run(["git", "rev-parse", "HEAD"]).trim(),
  sources: sources.map(source => ({ source, sha256: sha256(source) })) };
const report: any = { status: "running", startedAt: new Date().toISOString(), settings, host, identities,
  scope: "fresh Native CLI process: source TSX -> H264/AAC MP4 including finalization; peak process footprint",
  regressionTolerancePercent, rows: [], failures: [] };
const save = () => writeFile(path.join(output, "run.json"), JSON.stringify(report, null, 2) + "\n");
await save();

async function render(source: string, backend: string, round: number, trace: boolean) {
  const folder = path.join(output, path.basename(source, ".motion.tsx"), backend, trace ? "trace" : `round-${round}`);
  await mkdir(folder, { recursive: true });
  const video = path.join(folder, "export.mp4");
  const tracePath = path.join(folder, "trace.json");
  const cmd = [cli, "--json", "motion", "render", source, "-o", video, "--backend", backend,
    "--fps", String(settings.fps), "--output-size", `${settings.width}x${settings.height}`,
    "--workers", String(settings.workers), "--hardware-encode", "--bitrate", String(settings.bitrate)];
  const env = { ...process.env };
  for (const key of ["VALLE_RENDER_TRACE", "VALLE_PERF", "VALLE_TRACE_PIPELINE", "VALLE_COMPOSITOR_TRACE_PASSES", "VALLE_PERF_SCENE3D"]) delete env[key];
  if (trace) env.VALLE_RENDER_TRACE = tracePath;
  await writeFile(path.join(folder, "command.json"), JSON.stringify({ cmd, tracePath: trace ? tracePath : null }, null, 2));
  const started = performance.now();
  const child = Bun.spawn(["/usr/bin/time", "-l", ...cmd], { cwd: ROOT, env, stdout: "pipe", stderr: "pipe" });
  const [stdout, stderr, code] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited]);
  const wallMs = performance.now() - started;
  await writeFile(path.join(folder, "stdout.json"), stdout); await writeFile(path.join(folder, "stderr.log"), stderr);
  if (code !== 0) throw new Error(`render failed: ${source}/${backend}: ${stdout.slice(-1000)} ${stderr.slice(-1500)}`);
  const receipt = JSON.parse(stdout);
  if (receipt.delivery.backend !== backend) throw new Error("physical renderer drift");
  const streams = JSON.parse(run(["ffprobe", "-v", "error", "-count_frames", "-show_entries",
    "stream=codec_type,codec_name,width,height,avg_frame_rate,nb_read_frames,duration,sample_rate,channels,color_space,color_transfer,color_primaries,color_range", "-of", "json", video])).streams;
  const v = streams.find((stream: any) => stream.codec_type === "video");
  const a = streams.filter((stream: any) => stream.codec_type === "audio");
  if (v.codec_name !== "h264" || v.width !== settings.width || v.height !== settings.height
    || v.avg_frame_rate !== `${settings.fps}/1` || Number(v.nb_read_frames) !== settings.fps * settings.durationS
    || Math.abs(Number(v.duration) - settings.durationS) > 1 / settings.fps
    || a.length !== 1 || a[0].codec_name !== "aac" || a[0].sample_rate !== "48000" || a[0].channels !== 2) throw new Error("video/audio clock or format drift");
  run(["ffmpeg", "-v", "error", "-xerror", "-i", video, "-f", "null", "-"]);
  const stages: Record<string, number> = {};
  if (trace) {
    const data = await Bun.file(tracePath).json();
    if (data.metadata.droppedEvents) throw new Error("trace was truncated");
    for (const event of data.traceEvents) if (event.ph === "X") stages[event.name] = (stages[event.name] ?? 0) + event.dur / 1000;
  }
  const row = { source, backend, round, trace, wallMs, rtf: settings.durationS * 1000 / wallMs,
    peakFootprintBytes: Number(stderr.match(/(\d+)\s+peak memory footprint/)?.[1] ?? NaN),
    peakRssBytes: Number(stderr.match(/(\d+)\s+maximum resident set size/)?.[1] ?? NaN), stages, streams,
    folder: path.relative(output, folder) };
  if (!Number.isFinite(row.peakFootprintBytes) || !Number.isFinite(row.peakRssBytes)) throw new Error("missing memory measurement");
  report.rows.push(row); await save();
  console.log(`${path.basename(source)} ${backend} ${trace ? "trace" : round}: ${(wallMs / 1000).toFixed(3)}s ${(row.peakFootprintBytes / 1024 ** 2).toFixed(1)}MiB`);
}

try {
  for (let round = 0; round < settings.repeats; round++) for (const source of sources)
    for (let i = 0; i < backends.length; i++) await render(source, backends[(i + round) % backends.length]!, round, false);
  for (const source of sources) for (const backend of backends) await render(source, backend, 0, true);
  report.medians = sources.flatMap(source => backends.map(backend => {
    const rows = report.rows.filter((row: any) => row.source === source && row.backend === backend && !row.trace);
    return { source, backend, runs: rows.length, wallMs: median(rows.map((row: any) => row.wallMs)),
      peakFootprintBytes: median(rows.map((row: any) => row.peakFootprintBytes)) };
  }));
  if (args.includes("--compare")) {
    const before = await Bun.file(path.resolve(option(args, "--compare"))).json();
    if (before.status !== "passed" || JSON.stringify(before.settings) !== JSON.stringify(settings) || JSON.stringify(before.host) !== JSON.stringify(host)
      || JSON.stringify(before.identities.sources) !== JSON.stringify(identities.sources)) throw new Error("baseline settings/host/source differ");
    report.comparison = report.medians.map((row: any) => {
      const baseline = before.medians.find((previous: any) => previous.source === row.source && previous.backend === row.backend);
      if (!baseline || baseline.runs !== settings.repeats || ![baseline.wallMs, baseline.peakFootprintBytes].every(value => Number.isFinite(value) && value > 0)) throw new Error("baseline needs three successful runs");
      const wallChangePercent = (row.wallMs / baseline.wallMs - 1) * 100;
      const memoryChangePercent = (row.peakFootprintBytes / baseline.peakFootprintBytes - 1) * 100;
      const pass = wallChangePercent <= regressionTolerancePercent && memoryChangePercent <= regressionTolerancePercent;
      if (!pass) report.failures.push({ source: row.source, backend: row.backend, wallChangePercent, memoryChangePercent });
      return { ...row, wallChangePercent, memoryChangePercent, pass };
    });
  }
  report.status = report.failures.length ? "failed" : "passed";
  if (report.failures.length) process.exitCode = 1;
} catch (error) {
  report.status = "failed"; report.error = String(error); process.exitCode = 1;
} finally { report.completedAt = new Date().toISOString(); await save(); console.log(`Evidence: ${output}/run.json`); }
