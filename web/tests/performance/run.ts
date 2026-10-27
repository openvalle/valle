/** Fixed fixtures and settings; real devices, sequential measurements, persisted references. */
import { mkdir, writeFile, cp } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { spawnSync } from "node:child_process";
import { readPng } from "../parity/lib/png.ts";
import {
  resolveBrowser,
  browserLaunchCommand,
  terminateBrowser,
  waitForUnexpectedBrowserExit,
} from "../parity/lib/browser.ts";
import { compareFrames, crop, type PixelRule, type Region } from "./pixels.ts";
import { median, option, ROOT, sha256 } from "../../scripts/motion/common.ts";

const BASE = import.meta.dir;
type Case = {
  id: string;
  source: string;
  kind?: "timeline";
  group: string;
  assets: Record<string, string>;
  rule: PixelRule;
  repeatRule: PixelRule;
  backendRules?: Record<string, Partial<PixelRule>>;
  regions: Region[];
  order?: boolean;
};
const suite = (await Bun.file(path.join(BASE, "suite.json")).json()) as {
  qa: {
    width: number;
    height: number;
    frames: number[];
    order: number[];
    threadAllowances: number[];
    resourceCacheBytes: number;
    backendRules: Record<string, Partial<PixelRule>>;
  };
  order: { frames: number; stride: number };
  performance: {
    width: number;
    height: number;
    fps: number;
    durationS: number;
    repeats: number;
    threads: number;
    bitrate: number;
    profiles: Record<string, { width: number; height: number }>;
    regressionTolerance: { wallPercent: number; memoryPercent: number };
    target: { width: number; height: number; fps: number; rtfMedian: number };
  };
  cases: Case[];
};
const args = Bun.argv.slice(2);
if (args.includes("--help")) {
  console.log(
    "bun web/tests/performance/run.ts --output NEW_DIR [--mode qa|order|bench|all] [--cases ID,...] [--backends native-cpu,native-metal,web-cpu,web-gpu] [--profile preview|1080] [--target-4k] [--qa-4k --golden-dir DIR] [--browser-timeout-ms 300000] [--cpu-output-f32] [--codec h264|hevc] [--cli BINARY --fixed-runner BINARY --web-assets-dir DIR] [--compare BASELINE/run.json] [--update-golden]",
  );
  process.exit(0);
}
const output = path.resolve(option(args, "--output"));
const cli = path.resolve(option(args, "--cli", path.join(ROOT, "target/release/valle")));
const fixedRunner = path.resolve(
  option(
    args,
    "--fixed-runner",
    path.join(ROOT, "target/release/examples/performance_fixed_package"),
  ),
);
const webAssetsDir = path.resolve(option(args, "--web-assets-dir", path.join(ROOT, "web/dist")));
const mode = option(args, "--mode", "qa");
if (!["qa", "order", "bench", "all"].includes(mode)) throw new Error(`invalid mode ${mode}`);
const browserTimeoutMs = Number(option(args, "--browser-timeout-ms", "300000"));
if (
  !Number.isSafeInteger(browserTimeoutMs) ||
  browserTimeoutMs < 1000 ||
  browserTimeoutMs > 1_800_000
)
  throw new Error("browser timeout must be 1000..1800000 ms");
const memorySampleLimit = Math.ceil(browserTimeoutMs / 250) + 4;
// Diagnostic control: exercise the retained F32 API without changing product artifacts.
const cpuOutputF32 = args.includes("--cpu-output-f32");
// Native MP4 codec for benchmarks; browser exports stay H.264.
const nativeCodec = option(args, "--codec", "h264");
if (!["h264", "hevc"].includes(nativeCodec)) throw new Error(`invalid codec ${nativeCodec}`);
const selected = option(args, "--cases", suite.cases.map((c) => c.id).join(",")).split(",");
const cases = selected.map((id) => {
  const c = suite.cases.find((c) => c.id === id);
  if (!c) throw new Error(`unknown case ${id}`);
  return c;
});
const backends = option(args, "--backends", "native-cpu,native-metal,web-cpu,web-gpu").split(",");
if (backends.some((b) => !["native-cpu", "native-metal", "web-cpu", "web-gpu"].includes(b)))
  throw new Error("unknown backend");
if (await Bun.file(path.join(output, "run.json")).exists())
  throw new Error("result directory already used");
await mkdir(output, { recursive: true });
const goldenDir = path.resolve(option(args, "--golden-dir", path.join(BASE, "golden")));
if (args.includes("--qa-4k")) {
  if (goldenDir === path.join(BASE, "golden"))
    throw new Error(
      "4K QA requires a separate --golden-dir; preserve the registered preview references",
    );
  const scaleX = suite.performance.target.width / suite.qa.width;
  const scaleY = suite.performance.target.height / suite.qa.height;
  suite.qa.width = suite.performance.target.width;
  suite.qa.height = suite.performance.target.height;
  for (const c of cases)
    c.regions = c.regions.map((region) => ({
      ...region,
      x: region.x * scaleX,
      y: region.y * scaleY,
      width: region.width * scaleX,
      height: region.height * scaleY,
    }));
}
const assetArgs = (c: Case) =>
  Object.entries(c.assets).flatMap(([id, file]) => ["--asset", `${id}=${path.join(ROOT, file)}`]);
const source = (c: Case) => path.join(BASE, c.source);
/** Cross-backend tolerance: the case rule, then the suite's backend default, then the case's backend entry. */
const qaRule = (c: Case, backend: string): PixelRule => ({
  ...c.rule,
  ...suite.qa.backendRules[backend],
  ...c.backendRules?.[backend],
});
const identity = (c: Case) => ({
  sourceSha256: sha256(source(c)),
  assets: Object.fromEntries(
    Object.entries(c.assets).map(([k, p]) => [k, sha256(path.join(ROOT, p))]),
  ),
});
const profile = option(args, "--profile", "1080");
if (!suite.performance.profiles[profile]) throw new Error(`unknown performance profile ${profile}`);
const performance = {
  ...suite.performance,
  ...suite.performance.profiles[profile],
  ...(args.includes("--target-4k") ? suite.performance.target : {}),
};
const report: any = {
  startedAt: new Date().toISOString(),
  status: "running",
  host: {
    platform: os.platform(),
    arch: os.arch(),
    cpus: os.cpus().length,
    cpu: os.cpus()[0]?.model,
    totalMemory: os.totalmem(),
  },
  settings: { mode, backends, nativeCodec, qa: suite.qa, order: suite.order, performance },
  identities: {
    cli: sha256(cli),
    suite: sha256(path.join(BASE, "suite.json")),
    cases: Object.fromEntries(cases.map((c) => [c.id, identity(c)])),
  },
  qa: [],
  order: [],
  benchmarks: [],
  unmeasured: [],
  failures: [],
};
report.diagnostics = {
  browserTimeoutMs,
  cpuOutputRepresentation: cpuOutputF32 ? "forced-f32" : "product-default",
};
report.unmeasured.push(
  {
    metric: "GPU device execution time",
    reason: "CPU submit/completion waits are measured; no device timestamp queries",
  },
  {
    metric: "browser footprint / VRAM",
    reason: "process-tree RSS is sampled and reported separately; cache bytes are owner estimates",
  },
  {
    coverage: "HDR output, Scene3D, long-run steady memory",
    reason: "this fixed matrix is SDR 2D with 10-second exports; further suites required",
  },
  {
    metric: "browser hardware encoder identity",
    reason:
      "WebCodecs AVC uses the product's implementation default; physical encoder is not exposed",
  },
);
const saveReport = () =>
  writeFile(path.join(output, "run.json"), JSON.stringify(report, null, 2) + "\n");
await writeFile(path.join(output, "run.json"), JSON.stringify(report, null, 2));

function runSync(argv: string[], input?: Uint8Array) {
  const r = spawnSync(argv[0]!, argv.slice(1), { cwd: ROOT, input, maxBuffer: 64 * 1024 * 1024 });
  if (r.error || r.status !== 0)
    throw new Error(`${argv[0]} failed: ${r.stderr?.toString().slice(-3000)}`, { cause: r.error });
  return r.stdout;
}

async function native(c: Case, backend: string, argv: string[], folder: string, trace = false) {
  await mkdir(folder, { recursive: true });
  const tracePath = path.join(folder, "trace.json");
  let cmd = [
    cli,
    "--json",
    "motion",
    "render",
    source(c),
    ...assetArgs(c),
    "--backend",
    backend === "native-metal" ? "metal" : "raster",
    ...argv,
  ];
  let scope = "fresh CLI process: TSX -> MP4/PNG, includes finalization";
  let preparationMs: number | undefined;
  if (c.kind === "timeline") {
    const prepared = await resourceHost(c, folder);
    try {
      preparationMs = prepared.preparationMs;
      const size = option(argv, "--output-size").split("x").map(Number);
      const frames = argv.includes("--frames")
        ? option(argv, "--frames").split(",").map(Number)
        : null;
      const file = path.join(folder, "fixed-job.json");
      await writeFile(
        file,
        JSON.stringify(
          {
            ...prepared.config,
            localAssets: Object.values(c.assets).map((p) => path.join(ROOT, p)),
            backend,
            mode: frames ? "qa" : "bench",
            frames,
            width: size[0],
            height: size[1],
            threads: Number(option(argv, "--threads", "4")),
            bitrate: Number(option(argv, "--bitrate", String(performance.bitrate))),
            codec: nativeCodec,
            output: option(argv, frames ? "--storyboard" : "-o"),
          },
          null,
          2,
        ),
      );
      cmd = [fixedRunner, file];
      scope =
        "fresh native process: verified frozen Timeline package -> MP4/PNG; host preparation recorded separately";
    } finally {
      prepared.host.kill();
      await prepared.host.exited;
    }
  }
  const started = performanceNow();
  const env = { ...process.env };
  for (const key of [
    "VALLE_RENDER_TRACE",
    "VALLE_PERF",
    "VALLE_TRACE_PIPELINE",
    "VALLE_COMPOSITOR_TRACE_PASSES",
    "VALLE_PERF_SCENE3D",
  ])
    delete env[key];
  if (trace) env.VALLE_RENDER_TRACE = tracePath;
  // macOS time's footprint includes attributed GPU memory; RSS is reported separately.
  const timed = process.platform === "darwin" ? ["/usr/bin/time", "-l", ...cmd] : cmd;
  const child = Bun.spawn(timed, { cwd: ROOT, env, stdout: "pipe", stderr: "pipe" });
  const [stdout, stderr, code] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  const wallMs = performanceNow() - started;
  await writeFile(
    path.join(folder, "command.json"),
    JSON.stringify(
      { cmd, trace, scope, preparationMs, tracePath: trace ? tracePath : null },
      null,
      2,
    ),
  );
  await writeFile(path.join(folder, "stdout.json"), stdout);
  await writeFile(path.join(folder, "stderr.log"), stderr);
  if (code !== 0)
    throw new Error(`native ${c.id} ${backend}: ${stdout.slice(-1500)} ${stderr.slice(-2500)}`);
  const result = JSON.parse(stdout);
  if (result.delivery.backend !== (backend === "native-metal" ? "metal" : "raster"))
    throw new Error("native physical backend drift");
  const footprint = stderr.match(/(\d+)\s+peak memory footprint/);
  const rss = stderr.match(/(\d+)\s+maximum resident set size/);
  let traceResult;
  if (trace) {
    traceResult = await Bun.file(tracePath).json();
    if (traceResult.metadata.droppedEvents !== 0) throw new Error("native trace was truncated");
  }
  return {
    wallMs,
    result,
    scope,
    preparationMs,
    peakFootprintBytes: footprint ? Number(footprint[1]) : null,
    peakRssBytes: rss ? Number(rss[1]) : null,
    trace: traceResult,
  };
}

function performanceNow() {
  return globalThis.performance.now();
}

async function resourceHost(c: Case, folder: string) {
  const started = performanceNow();
  const host = Bun.spawn(
    [
      cli,
      "--json",
      c.kind === "timeline" ? "timeline" : "motion",
      "studio",
      source(c),
      ...(c.kind === "timeline" ? [] : assetArgs(c)),
      "--port",
      "0",
      "--web-assets-dir",
      webAssetsDir,
    ],
    { cwd: ROOT, stdout: "pipe", stderr: Bun.file(path.join(folder, "host.stderr.log")) },
  );
  try {
    const reader = host.stdout.getReader();
    let line = "";
    while (!line.includes("\n")) {
      const p = await reader.read();
      if (p.done) throw new Error("resource host exited before ready");
      line += new TextDecoder().decode(p.value);
    }
    reader.releaseLock();
    const upstream = new URL(JSON.parse(line.split("\n")[0]!).url).origin;
    let config = await (await fetch(`${upstream}/config.json`)).json();
    if (c.kind === "timeline") {
      const boot = await (await fetch(`${upstream}/studio/boot.json`)).json();
      const author = await Bun.file(source(c)).json();
      const factsResponse = await fetch(`${upstream}/timeline/media-facts`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-valle-token": boot.session.token,
          origin: upstream,
        },
        body: JSON.stringify({ timeline: author }),
      });
      const facts = await factsResponse.json();
      if (!factsResponse.ok || facts.status !== "ok")
        throw new Error(`media admission failed: ${JSON.stringify(facts)}`);
      const wasm = await import("../../packages/engine/generated/web/valle_engine.js");
      wasm.initSync({
        module: await Bun.file(
          path.join(ROOT, "web/packages/engine/generated/web/valle_engine_bg.wasm"),
        ).arrayBuffer(),
      });
      const { preparePreviewPackageWithWasm } =
        await import("../../packages/engine/src/compiler.ts");
      const prepared = preparePreviewPackageWithWasm(wasm, {
        authorTimeline: author,
        motionInstances: [],
        resourceInputs: facts.resources,
        resourceBytes: facts.resourceBytes
          ? new Uint8Array(
              await (await fetch(new URL(facts.resourceBytes.url, upstream))).arrayBuffer(),
            )
          : new Uint8Array(),
      });
      const urls = boot.runtime.assetUrls;
      config = {
        ...prepared,
        assets: facts.assets,
        assetBaseUrl: boot.runtime.assetBaseUrl,
        runtimeAssets: {
          engine: { glue: urls.engineGlue, wasm: urls.engineWasm },
          canvasKit: { glue: urls.canvasKitGlue, wasm: urls.canvasKitWasm },
          workers: { productFrame: urls.productFrameWorker },
        },
      };
      await writeFile(path.join(folder, "media-facts.json"), JSON.stringify(facts, null, 2));
    }
    if (!config.fixedPackageManifestJson)
      throw new Error(`package preparation failed: ${JSON.stringify(config)}`);
    if (!config.resourceBytes && config.resourceBytesUrl)
      config.resourceBytes = new Uint8Array(
        await (await fetch(new URL(config.resourceBytesUrl, upstream))).arrayBuffer(),
      );
    const { resourceBytes, ...metadata } = config;
    await writeFile(path.join(folder, "resource-bytes.bin"), resourceBytes ?? new Uint8Array());
    await writeFile(
      path.join(folder, "package.json"),
      JSON.stringify({ ...metadata, resourceBytesUrl: "resource-bytes.bin" }, null, 2),
    );
    return { host, upstream, config, preparationMs: performanceNow() - started };
  } catch (error) {
    host.kill();
    await host.exited;
    throw error;
  }
}

async function browserJob(c: Case, backend: string, job: Record<string, unknown>, folder: string) {
  const browser = resolveBrowser();
  if (!browser) throw new Error("Chromium unavailable");
  await mkdir(folder, { recursive: true });
  // Reuse the product's frozen preparation and resource host; rendering/encoding is browser-only.
  const { host, upstream, config, preparationMs } = await resourceHost(c, folder);
  let child: Bun.Subprocess | undefined;
  let server: ReturnType<typeof Bun.serve> | undefined;
  let completed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let memoryTimer: ReturnType<typeof setInterval> | undefined;
  const memorySamples: { timeMs: number; processes: number; rssBytes: number }[] = [];
  try {
    const build = await Bun.build({
      entrypoints: [path.join(BASE, "browser-client.ts")],
      target: "browser",
      format: "esm",
    });
    if (!build.success) throw new Error(build.logs.map((l) => l.message).join("\n"));
    let resolveResult!: (v: any) => void;
    let rejectResult!: (e: unknown) => void;
    const resultPromise = new Promise<any>((resolve, reject) => {
      resolveResult = resolve;
      rejectResult = reject;
    });
    const uploads: Promise<unknown>[] = [];
    server = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      async fetch(request) {
        const url = new URL(request.url);
        if (url.pathname === "/")
          return new Response(
            '<!doctype html><meta charset="utf-8"><script>addEventListener("error",e=>fetch("/result",{method:"POST",body:JSON.stringify({status:"failed",error:e.message})}));addEventListener("unhandledrejection",e=>fetch("/result",{method:"POST",body:JSON.stringify({status:"failed",error:String(e.reason)})}));</script><script type="module" src="/client.js"></script>',
            { headers: { "content-type": "text/html" } },
          );
        if (url.pathname === "/client.js")
          return new Response(build.outputs[0], { headers: { "content-type": "text/javascript" } });
        if (url.pathname === "/job.json")
          return Response.json({
            ...job,
            gpu: backend === "web-gpu",
            cpuOutputF32,
            input: {
              ...config,
              resourceBytes: undefined,
              resourceBytesUrl: "/resource-bytes.bin",
              assets: config.resourceLocators ?? config.assets,
            },
          });
        if (url.pathname === "/resource-bytes.bin")
          return new Response(config.resourceBytes ?? new Uint8Array(), {
            headers: { "content-type": "application/octet-stream" },
          });
        if (url.pathname === "/result" && request.method === "POST") {
          resolveResult(await request.json());
          return new Response("ok");
        }
        if (url.pathname.startsWith("/artifact/") && request.method === "POST") {
          const name = url.pathname.slice(10);
          if (!/^[\w.-]+\.(png|mp4)$/.test(name))
            return new Response("invalid name", { status: 400 });
          const write = Bun.write(path.join(folder, name), await request.arrayBuffer());
          uploads.push(write);
          await write;
          return new Response("ok");
        }
        if (
          ["/packages/", "/chunks/", "/runtime/"].some((prefix) => url.pathname.startsWith(prefix))
        ) {
          const dist = webAssetsDir;
          const file = path.resolve(dist, `.${url.pathname}`);
          if (!file.startsWith(dist + path.sep))
            return new Response("invalid path", { status: 400 });
          const asset = Bun.file(file);
          if (await asset.exists()) {
            if (
              cpuOutputF32 &&
              url.pathname === new URL(config.runtimeAssets.engine.glue, server!.url).pathname
            ) {
              const glue = await asset.text();
              if (!glue.includes("export class ProductEngine"))
                throw new Error("unexpected Engine glue for F32 control");
              return new Response(
                glue + "\nProductEngine.prototype.transform_srgb_preview_f16 = undefined;\n",
                { headers: { "content-type": "text/javascript" } },
              );
            }
            return new Response(asset);
          }
        }
        // The resource host validates its own port. Forwarding the browser's proxy Host
        // header rejects otherwise valid image/video requests before asset lookup.
        const headers = new Headers(request.headers);
        headers.delete("host");
        return fetch(`${upstream}${url.pathname}${url.search}`, { headers });
      },
    });
    const profile = path.join(folder, "chrome-profile");
    const cmd = [
      ...browserLaunchCommand(browser),
      "--headless=new",
      "--enable-gpu",
      "--disable-software-rasterizer",
      ...(process.platform === "darwin" ? ["--use-angle=metal"] : []),
      "--disable-background-networking",
      "--disable-extensions",
      "--no-first-run",
      "--no-default-browser-check",
      `--user-data-dir=${profile}`,
      server.url.toString(),
    ];
    await writeFile(path.join(folder, "command.json"), JSON.stringify({ cmd, job }, null, 2));
    child = Bun.spawn(cmd, {
      stdout: "ignore",
      stderr: Bun.file(path.join(folder, "chrome.stderr.log")),
    });
    const browserStarted = performanceNow();
    const browserPid = child.pid;
    const sampleMemory = () => {
      // RSS includes shared pages in more than one process. Keep its scope separate from footprint.
      const ps = spawnSync("ps", ["-axo", "pid=,ppid=,rss="], { encoding: "utf8" });
      if (ps.status !== 0) return;
      const rows = ps.stdout
        .trim()
        .split("\n")
        .map((line) => line.trim().split(/\s+/).map(Number));
      const pids = new Set([browserPid]);
      for (let size = -1; size !== pids.size;) {
        size = pids.size;
        for (const [pid, ppid] of rows) if (pids.has(ppid!)) pids.add(pid!);
      }
      const alive = rows.filter(([pid]) => pids.has(pid!));
      if (memorySamples.length < memorySampleLimit)
        memorySamples.push({
          timeMs: performanceNow() - browserStarted,
          processes: alive.length,
          rssBytes: alive.reduce((sum, row) => sum + row[2]! * 1024, 0),
        });
    };
    sampleMemory();
    memoryTimer = setInterval(sampleMemory, 250);
    timer = setTimeout(
      () => rejectResult(new Error(`browser job timed out after ${browserTimeoutMs} ms`)),
      browserTimeoutMs,
    );
    const result = await Promise.race([
      resultPromise,
      waitForUnexpectedBrowserExit(child, {
        operation: "performance baseline",
        binary: browser,
        pageUrl: server.url.toString(),
        stderrPath: path.join(folder, "chrome.stderr.log"),
        isCompleted: () => completed,
      }),
    ]);
    completed = true;
    await Promise.all(uploads);
    clearInterval(memoryTimer);
    memoryTimer = undefined;
    sampleMemory();
    await writeFile(
      path.join(folder, "memory.json"),
      JSON.stringify(
        {
          scope:
            "sampled Chrome process-tree RSS; shared pages may be counted multiple times; not footprint or device allocation",
          intervalMs: 250,
          samples: memorySamples,
        },
        null,
        2,
      ),
    );
    result.peakProcessTreeRssBytes = memorySamples.length
      ? Math.max(...memorySamples.map((s) => s.rssBytes))
      : null;
    result.memorySamples = memorySamples.length;
    await writeFile(path.join(folder, "browser.json"), JSON.stringify(result, null, 2));
    if (result.status !== "ok") throw new Error(result.error ?? "browser job failed");
    if (
      backend === "web-gpu" &&
      process.platform === "darwin" &&
      !/Apple.*Metal/.test(result.gpu.renderer)
    )
      throw new Error(`expected Apple Metal GPU: ${result.gpu.renderer}`);
    if (result.trace) {
      if (result.trace.metadata.droppedEvents !== 0) throw new Error("browser trace was truncated");
      await writeFile(path.join(folder, "trace.json"), JSON.stringify(result.trace));
    }
    return { ...result, preparationMs };
  } finally {
    completed = true;
    if (timer) clearTimeout(timer);
    if (memoryTimer) clearInterval(memoryTimer);
    await writeFile(
      path.join(folder, "memory.json"),
      JSON.stringify(
        {
          scope:
            "sampled Chrome process-tree RSS; shared pages may be counted multiple times; not footprint or device allocation",
          intervalMs: 250,
          samples: memorySamples,
        },
        null,
        2,
      ),
    );
    if (child) await terminateBrowser(child);
    server?.stop(true);
    host.kill();
    await host.exited;
  }
}

async function png(
  image: { width: number; height: number; data: Uint8Array | Uint8ClampedArray },
  file: string,
) {
  runSync(
    [
      "ffmpeg",
      "-v",
      "error",
      "-f",
      "rawvideo",
      "-pix_fmt",
      "rgba",
      "-s",
      `${image.width}x${image.height}`,
      "-i",
      "pipe:0",
      "-frames:v",
      "1",
      "-pix_fmt",
      "rgba",
      file,
    ],
    Uint8Array.from(image.data),
  );
}

async function qaCase(c: Case, backend: string, golden: any) {
  const folder = path.join(output, "qa", backend, c.id);
  const references = await readPng(path.join(goldenDir, `${c.id}.png`));
  const { width, height, frames, order } = suite.qa;
  const saved = golden.cases[c.id];
  if (JSON.stringify(saved.identity) !== JSON.stringify(identity(c)))
    throw new Error(`golden source/resource identity changed: ${c.id}`);
  if (
    JSON.stringify(saved.rule) !== JSON.stringify(c.rule) ||
    JSON.stringify(saved.repeatRule) !== JSON.stringify(c.repeatRule) ||
    JSON.stringify(saved.backendRules ?? {}) !== JSON.stringify(c.backendRules ?? {})
  )
    throw new Error(`registered pixel tolerance changed: ${c.id}`);
  const rule = qaRule(c, backend);
  if (sha256(path.join(goldenDir, `${c.id}.png`)) !== saved.pngSha256)
    throw new Error(`golden pixels changed: ${c.id}`);
  const rows: any[] = [];
  const cold = new Map<number, ReturnType<typeof crop>>();
  const compare = async (actual: ReturnType<typeof crop>, frame: number, label: string) => {
    const index = frames.indexOf(frame);
    if (index < 0) throw new Error("unexpected frame key");
    const expected = crop(references, index * width, 0, width, height);
    const result = compareFrames(expected, actual, rule);
    const regions = c.regions.map((r) => {
      const { diff: _diff, ...m } = compareFrames(expected, actual, rule, r);
      return m;
    });
    const old = cold.get(frame);
    const repeat = old ? compareFrames(old, actual, c.repeatRule) : null;
    if (!old) cold.set(frame, actual);
    rows.push({
      frame,
      label,
      ...result,
      diff: undefined,
      repeat: repeat ? { ...repeat, diff: undefined } : null,
      regions,
    });
    if (!result.pass || (repeat && !repeat.pass)) {
      await png(expected, path.join(folder, `${label}.reference.png`));
      await png(actual, path.join(folder, `${label}.actual.png`));
      await png(
        repeat && !repeat.pass ? repeat.diff : result.diff,
        path.join(folder, `${label}.diff.png`),
      );
    }
  };
  if (backend.startsWith("native")) {
    for (const threads of suite.qa.threadAllowances) {
      const f = path.join(folder, `threads-${threads}`);
      const sheet = path.join(f, "sheet.png");
      await native(
        c,
        backend,
        [
          "--frames",
          order.join(","),
          "--storyboard",
          sheet,
          "--output-size",
          `${width}x${height}`,
          "--threads",
          String(threads),
        ],
        f,
        true,
      );
      const image = await readPng(sheet);
      const columns = Math.min(4, order.length);
      for (const [i, frame] of order.entries())
        await compare(
          crop(image, (i % columns) * width, Math.floor(i / columns) * height, width, height),
          frame,
          `n${threads}-${i}`,
        );
    }
  } else {
    const r = await browserJob(
      c,
      backend,
      {
        mode: "qa",
        width,
        height,
        order,
        resourceCacheBytes: suite.qa.resourceCacheBytes,
        trace: true,
      },
      folder,
    );
    for (const entry of r.outputs)
      if (entry.name)
        await compare(
          await readPng(path.join(folder, entry.name)),
          entry.frame,
          entry.name.replace(/\.png$/, ""),
        );
  }
  const pass = rows.every((r) => r.pass && (!r.repeat || r.repeat.pass));
  await writeFile(
    path.join(folder, "pixels.json"),
    JSON.stringify({ pass, rule, repeatRule: c.repeatRule, rows }, null, 2),
  );
  report.qa.push({
    case: c.id,
    backend,
    pass,
    rule,
    // Same-backend optimization comparisons must not inherit looser cross-backend RGB
    // exceptions (for example, particles). Only the registered geometric edge allowance applies.
    optimizationRule: { ...c.rule, edge: suite.qa.backendRules[backend]?.edge ?? 0 },
    comparisons: rows.length,
    maxRgb: Math.max(...rows.map((r) => r.rgbMax)),
    maxAlpha: Math.max(...rows.map((r) => r.alphaMax)),
    maximumEdgePixels: Math.max(...rows.map((r) => r.edgePixels)),
    maximumRepeatRgb: Math.max(...rows.map((r) => r.repeat?.rgbMax ?? 0)),
    maximumRepeatAlpha: Math.max(...rows.map((r) => r.repeat?.alphaMax ?? 0)),
    maximumRepeatWhite: Math.max(...rows.map((r) => r.repeat?.whiteMax ?? 0)),
  });
  if (!pass)
    report.failures.push({
      case: c.id,
      backend,
      kind: "pixels",
      evidence: path.relative(output, folder),
    });
  await saveReport();
  console.log(`QA ${backend} ${c.id}: ${pass ? "PASS" : "FAIL"}`);
}

/** Render the same frames in playback order and in a strided order from cold sessions. A frame is a
 * function of its inputs, so any difference from the playback-order capture comes from history. */
async function orderCase(c: Case, backend: string) {
  const folder = path.join(output, "order", backend, c.id);
  const { width, height } = suite.qa,
    { stride } = suite.order;
  const frames = Array.from({ length: suite.order.frames }, (_, i) => i);
  const orders: Record<string, { frames: number[]; threads: number }> = {
    sequential: { frames, threads: 1 },
    strided: {
      frames: Array.from({ length: stride }, (_, start) =>
        frames.filter((f) => f % stride === start),
      ).flat(),
      threads: 1,
    },
  };
  if (backend.startsWith("native")) orders["sequential-n4"] = { frames, threads: 4 };
  const captures = new Map<string, Map<number, ReturnType<typeof crop>>>();
  let sessions: any[] = [];
  if (backend.startsWith("native")) {
    for (const [label, { frames: order, threads }] of Object.entries(orders)) {
      const f = path.join(folder, label);
      const sheet = path.join(f, "sheet.png");
      await native(
        c,
        backend,
        [
          "--frames",
          order.join(","),
          "--storyboard",
          sheet,
          "--output-size",
          `${width}x${height}`,
          "--threads",
          String(threads),
        ],
        f,
      );
      const image = await readPng(sheet);
      const columns = Math.min(4, order.length);
      captures.set(
        label,
        new Map(
          order.map((frame, i) => [
            frame,
            crop(image, (i % columns) * width, Math.floor(i / columns) * height, width, height),
          ]),
        ),
      );
    }
  } else {
    const r = await browserJob(
      c,
      backend,
      {
        mode: "order",
        width,
        height,
        orders: Object.fromEntries(Object.entries(orders).map(([label, o]) => [label, o.frames])),
      },
      folder,
    );
    for (const label of Object.keys(orders)) captures.set(label, new Map());
    for (const entry of r.outputs)
      if (entry.name)
        captures.get(entry.order)!.set(entry.frame, await readPng(path.join(folder, entry.name)));
    sessions = r.outputs.filter((entry: any) => entry.session);
  }
  const reference = captures.get("sequential")!,
    rows: any[] = [];
  for (const [label, images] of captures) {
    if (label === "sequential") continue;
    const results = frames.map((frame) => ({
      frame,
      ...compareFrames(reference.get(frame)!, images.get(frame)!, c.repeatRule),
    }));
    const differing = results.filter((r) => !r.pass),
      worst = [...differing].sort((a, b) => b.failedPixels - a.failedPixels).slice(0, 3);
    for (const r of worst) {
      await png(reference.get(r.frame)!, path.join(folder, `${label}-${r.frame}.sequential.png`));
      await png(images.get(r.frame)!, path.join(folder, `${label}-${r.frame}.actual.png`));
      await png(r.diff, path.join(folder, `${label}-${r.frame}.diff.png`));
    }
    rows.push({
      order: label,
      frames: frames.length,
      differingFrames: differing.length,
      failedPixels: differing.reduce((sum, r) => sum + r.failedPixels, 0),
      maxRgb: Math.max(...results.map((r) => r.rgbMax)),
      maxAlpha: Math.max(...results.map((r) => r.alphaMax)),
      maxWhite: Math.max(...results.map((r) => r.whiteMax)),
      worst: worst.map((r) => ({
        frame: r.frame,
        failedPixels: r.failedPixels,
        rgbMax: r.rgbMax,
        alphaMax: r.alphaMax,
      })),
    });
  }
  const memory = Object.fromEntries(
    sessions.map((s) => [
      s.session,
      Object.fromEntries(
        [
          "physicalSurfaces",
          "maximumSurfaceResidentBytes",
          "maximumSurfacePeakBytes",
          "maximumSurfacePeakBytesByKind",
          "maximumPlannedSurfacePeakBytes",
          "maximumSurfaceEnvelopeBytes",
          "maximumSurfaceOverflowBytes",
          "surfaceOverflowFrames",
          "maximumAdmittedFrameBytes",
          "maximumGpuCacheUsageBytes",
          "maximumGpuCacheLimitBytes",
          "maximumGeometryTargetBytes",
        ].map((k) => [k, s.stats?.[k] ?? null]),
      ),
    ]),
  );
  const pass = rows.every((r) => r.differingFrames === 0);
  await writeFile(
    path.join(folder, "order.json"),
    JSON.stringify({ pass, repeatRule: c.repeatRule, orders, rows, sessions }, null, 2),
  );
  report.order.push({ case: c.id, backend, pass, rows, memory });
  if (!pass)
    report.failures.push({
      case: c.id,
      backend,
      kind: "order",
      evidence: path.relative(output, folder),
    });
  await saveReport();
  console.log(
    `ORDER ${backend} ${c.id}: ${pass ? "PASS" : "FAIL"} ${rows.map((r) => `${r.order} ${r.differingFrames}/${r.frames} max ${r.maxRgb.toFixed(1)}`).join(", ")}`,
  );
}

function validateVideo(file: string, backend: string) {
  const streams = JSON.parse(
    runSync([
      "ffprobe",
      "-v",
      "error",
      "-count_frames",
      "-show_entries",
      "stream=codec_type,codec_name,codec_tag_string,width,height,avg_frame_rate,nb_read_frames,duration,sample_rate,channels",
      "-of",
      "json",
      file,
    ]).toString(),
  ).streams;
  const value = streams.find((s: any) => s.codec_type === "video");
  const codec = backend.startsWith("native") ? nativeCodec : "h264";
  if (
    value.codec_name !== codec ||
    (codec === "hevc" && value.codec_tag_string !== "hvc1") ||
    value.width !== performance.width ||
    value.height !== performance.height ||
    Number(value.nb_read_frames) !== 600 ||
    value.avg_frame_rate !== "60/1" ||
    Math.abs(Number(value.duration) - 10) > 1 / 60
  )
    throw new Error(`video metadata drift: ${file} ${JSON.stringify(value)}`);
  const audio = streams.filter((s: any) => s.codec_type === "audio");
  if (
    backend.startsWith("native")
      ? audio.length !== 1 ||
        audio[0].codec_name !== "aac" ||
        audio[0].sample_rate !== "48000" ||
        audio[0].channels !== 2
      : audio.length !== 0
  )
    throw new Error(`audio policy drift: ${file} ${JSON.stringify(audio)}`);
  runSync(["ffmpeg", "-v", "error", "-xerror", "-i", file, "-f", "null", "-"]);
  return { ...value, audio };
}

async function benchCase(c: Case, backend: string, round: number, trace: boolean) {
  const folder = path.join(output, "bench", backend, c.id, trace ? "trace" : `round-${round}`);
  let result: any;
  if (backend.startsWith("native")) {
    const video = path.join(folder, "export.mp4");
    result = await native(
      c,
      backend,
      [
        "-o",
        video,
        "--fps",
        "60",
        "--output-size",
        `${performance.width}x${performance.height}`,
        "--threads",
        String(performance.threads),
        "--hardware-encode",
        "--bitrate",
        String(performance.bitrate),
        ...(nativeCodec === "hevc" ? ["--codec", "hevc"] : []),
      ],
      folder,
      trace,
    );
  } else {
    result = await browserJob(
      c,
      backend,
      {
        mode: "bench",
        width: performance.width,
        height: performance.height,
        bitrate: performance.bitrate,
        trace,
      },
      folder,
    );
  }
  const video = validateVideo(path.join(folder, "export.mp4"), backend);
  const stages: Record<string, number> = {};
  if (result.trace)
    for (const event of result.trace.traceEvents)
      if (event.ph === "X") stages[event.name] = (stages[event.name] ?? 0) + event.dur / 1000;
  const row = {
    case: c.id,
    group: c.group,
    backend,
    round,
    trace,
    wallMs: result.wallMs,
    fps: 600000 / result.wallMs,
    rtf: 10000 / result.wallMs,
    peakFootprintBytes: result.peakFootprintBytes ?? null,
    peakRssBytes: result.peakRssBytes ?? null,
    peakProcessTreeRssBytes: result.peakProcessTreeRssBytes ?? null,
    preparationMs: result.preparationMs ?? null,
    scope: result.scope ?? "fresh CLI process TSX -> MP4",
    video,
    stages,
    folder: path.relative(output, folder),
  };
  report.benchmarks.push(row);
  console.log(
    `BENCH ${backend} ${c.id} ${trace ? "trace" : round}: ${(row.wallMs / 1000).toFixed(3)}s RTF=${row.rtf.toFixed(2)}`,
  );
  await saveReport();
}

try {
  report.identities.gitHead = runSync(["git", "rev-parse", "HEAD"]).toString().trim();
  report.identities.gitStatus = runSync(["git", "status", "--short"]).toString();
  report.identities.wasm = sha256(
    path.join(ROOT, "web/packages/engine/generated/web/valle_engine_bg.wasm"),
  );
  report.identities.webAssetsDir = webAssetsDir;
  report.identities.runtime = sha256(path.join(webAssetsDir, "packages/engine/index.mjs"));
  report.identities.runtimeManifest = sha256(
    path.join(webAssetsDir, "runtime/manifests/engine.json"),
  );
  // Browser jobs load the served distribution; generated artifacts can belong to a different
  // build while native binaries or old runtime snapshots are being compared.
  report.identities.servedWasm = sha256(
    path.join(webAssetsDir, "runtime/engine/valle_engine_bg.wasm"),
  );
  report.identities.servedEngineGlue = sha256(
    path.join(webAssetsDir, "runtime/engine/valle_engine.js"),
  );
  report.identities.runner = sha256(import.meta.path);
  report.identities.browserClient = sha256(path.join(BASE, "browser-client.ts"));
  report.identities.fixedRunner = (await Bun.file(fixedRunner).exists())
    ? sha256(fixedRunner)
    : null;
  if (args.includes("--update-golden")) {
    await mkdir(goldenDir, { recursive: true });
    const golden: any = (await Bun.file(path.join(goldenDir, "manifest.json")).exists())
      ? await Bun.file(path.join(goldenDir, "manifest.json")).json()
      : {
          createdAt: new Date().toISOString(),
          reference: "Native Raster",
          code: report.identities.gitHead,
          qa: suite.qa,
          cases: {},
        };
    if (JSON.stringify(golden.qa) !== JSON.stringify(suite.qa))
      throw new Error("golden QA settings differ; use a separate reference directory");
    for (const c of cases) {
      const folder = path.join(output, "golden", c.id),
        sheet = path.join(folder, "sheet.png");
      await native(
        c,
        "native-cpu",
        [
          "--frames",
          suite.qa.frames.join(","),
          "--storyboard",
          sheet,
          "--output-size",
          `${suite.qa.width}x${suite.qa.height}`,
          "--threads",
          "1",
        ],
        folder,
      );
      await cp(sheet, path.join(goldenDir, `${c.id}.png`));
      golden.cases[c.id] = {
        identity: identity(c),
        pngSha256: sha256(sheet),
        rule: c.rule,
        repeatRule: c.repeatRule,
        backendRules: c.backendRules ?? {},
        generatedAt: new Date().toISOString(),
        generator: {
          gitHead: report.identities.gitHead,
          cli: report.identities.cli,
          fixedRunner: report.identities.fixedRunner,
        },
      };
    }
    await writeFile(path.join(goldenDir, "manifest.json"), JSON.stringify(golden, null, 2) + "\n");
  }
  if (mode === "qa" || mode === "all") {
    const golden = await Bun.file(path.join(goldenDir, "manifest.json")).json();
    if (JSON.stringify(golden.qa) !== JSON.stringify(suite.qa))
      throw new Error("golden QA settings differ from requested resolution/order");
    for (const c of cases)
      for (const backend of backends) {
        if (backend === "native-metal" && process.platform !== "darwin") {
          report.unmeasured.push({ backend, reason: "Metal requires macOS" });
          continue;
        }
        if (backend.startsWith("web") && !resolveBrowser()) {
          report.unmeasured.push({ backend, reason: "Chromium unavailable" });
          continue;
        }
        try {
          await qaCase(c, backend, golden);
        } catch (error) {
          report.failures.push({ case: c.id, backend, error: String(error) });
          console.log(`QA ${backend} ${c.id}: ERROR ${String(error)}`);
          await saveReport();
        }
      }
  }
  if (mode === "order" || mode === "all") {
    for (const c of cases.filter((c) => c.order))
      for (const backend of backends) {
        if (backend === "native-metal" && process.platform !== "darwin") {
          report.unmeasured.push({ backend, reason: "Metal requires macOS" });
          continue;
        }
        if (backend.startsWith("web") && !resolveBrowser()) {
          report.unmeasured.push({ backend, reason: "Chromium unavailable" });
          continue;
        }
        try {
          await orderCase(c, backend);
        } catch (error) {
          report.failures.push({ case: c.id, backend, kind: "order", error: String(error) });
          console.log(`ORDER ${backend} ${c.id}: ERROR ${String(error)}`);
          await saveReport();
        }
      }
  }
  if (mode === "bench" || mode === "all") {
    const unavailable = new Map<string, string>();
    if (process.platform !== "darwin") unavailable.set("native-metal", "Metal requires macOS");
    if (!resolveBrowser())
      for (const backend of ["web-cpu", "web-gpu"])
        unavailable.set(backend, "Chromium unavailable");
    for (const [backend, reason] of unavailable)
      if (backends.includes(backend)) report.unmeasured.push({ backend, reason });
    // Rotate backend order each round; traced runs are diagnostics and excluded from medians.
    for (let round = 0; round < performance.repeats; round++)
      for (const c of cases)
        for (let i = 0; i < backends.length; i++) {
          const backend = backends[(i + round) % backends.length]!;
          if (unavailable.has(backend)) continue;
          try {
            await benchCase(c, backend, round, false);
          } catch (error) {
            report.failures.push({ case: c.id, backend, round, error: String(error) });
            console.log(`BENCH ERROR ${c.id} ${backend}: ${error}`);
            await saveReport();
          }
        }
    for (const c of cases)
      for (const backend of backends) {
        if (unavailable.has(backend)) continue;
        try {
          await benchCase(c, backend, 0, true);
        } catch (error) {
          report.failures.push({ case: c.id, backend, trace: true, error: String(error) });
        }
      }
    report.medians = cases.flatMap((c) =>
      backends.map((backend) => {
        const rows = report.benchmarks.filter(
          (r: any) => r.case === c.id && r.backend === backend && !r.trace,
        );
        return {
          case: c.id,
          group: c.group,
          backend,
          runs: rows.length,
          wallMs: rows.length ? median(rows.map((r: any) => r.wallMs)) : null,
          rtf: rows.length ? median(rows.map((r: any) => r.rtf)) : null,
          peakFootprintBytes:
            rows[0]?.peakFootprintBytes === null
              ? null
              : rows.length
                ? median(rows.map((r: any) => r.peakFootprintBytes))
                : null,
          peakProcessTreeRssBytes:
            rows[0]?.peakProcessTreeRssBytes === null
              ? null
              : rows.length
                ? median(rows.map((r: any) => r.peakProcessTreeRssBytes))
                : null,
        };
      }),
    );
    if (args.includes("--target-4k"))
      report.targetEvaluation = {
        rtfThreshold: suite.performance.target.rtfMedian,
        scope: "selected regular fixtures and requested backends only",
        rows: report.medians
          .filter((m: any) => m.group === "regular")
          .map((m: any) => ({
            case: m.case,
            backend: m.backend,
            rtf: m.rtf,
            targetMet:
              m.runs === performance.repeats ? m.rtf >= suite.performance.target.rtfMedian : null,
          })),
      };
    if (args.includes("--compare")) {
      const baseline = await Bun.file(path.resolve(option(args, "--compare"))).json();
      if (
        JSON.stringify(baseline.host) !== JSON.stringify(report.host) ||
        JSON.stringify(baseline.settings.performance) !==
          JSON.stringify(report.settings.performance) ||
        (baseline.settings.nativeCodec ?? "h264") !== nativeCodec
      )
        throw new Error("baseline hardware/settings differ");
      for (const c of cases)
        if (
          JSON.stringify(baseline.identities.cases[c.id]) !==
          JSON.stringify(report.identities.cases[c.id])
        )
          throw new Error(`baseline fixture differs: ${c.id}`);
      const comparisons: any[] = [];
      for (const current of report.medians) {
        const before = baseline.medians?.find(
          (r: any) => r.case === current.case && r.backend === current.backend,
        );
        if (!before || before.runs !== performance.repeats || current.runs !== performance.repeats)
          throw new Error(
            `baseline needs ${performance.repeats} successful untraced runs: ${current.case}/${current.backend}`,
          );
        for (const [metric, percent] of [
          ["wallMs", performance.regressionTolerance.wallPercent],
          ["peakFootprintBytes", performance.regressionTolerance.memoryPercent],
          ["peakProcessTreeRssBytes", performance.regressionTolerance.memoryPercent],
        ] as const) {
          if (before[metric] === null || current[metric] === null) continue;
          const changePercent = (current[metric] / before[metric] - 1) * 100;
          const pass = changePercent <= percent;
          comparisons.push({
            case: current.case,
            backend: current.backend,
            metric,
            before: before[metric],
            current: current[metric],
            changePercent,
            tolerancePercent: percent,
            pass,
          });
          if (!pass)
            report.failures.push({
              kind: "performance-regression",
              case: current.case,
              backend: current.backend,
              metric,
              changePercent,
              tolerancePercent: percent,
            });
        }
      }
      report.comparison = { baseline: path.resolve(option(args, "--compare")), rows: comparisons };
    }
  }
  report.status = report.failures.length
    ? "failed"
    : report.unmeasured.some((u: any) => u.backend)
      ? "partial"
      : "passed";
} catch (error) {
  report.status = "failed";
  report.failures.push({ error: String(error) });
} finally {
  report.finishedAt = new Date().toISOString();
  await writeFile(path.join(output, "run.json"), JSON.stringify(report, null, 2) + "\n");
}
console.log(`Evidence: ${output}/run.json`);
if (report.status !== "passed") process.exitCode = 1;
