/** Repeat B01 compile, preparation, and render measurements on one machine. */
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import * as os from "node:os";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { median, nonnegativeInteger, option, ROOT, run, runJson, sha256 } from "./common.ts";

const fixture = join(ROOT, "crates/valle-compiler/tests/fixtures/motion/composition/grid-instances.motion.tsx");

type CheckReport = {
  expressions: number;
  templates: number;
  instanceRows: number;
  timings: { templateCompile: number; instanceData: number; framePrepare: number };
};
type RenderTiming = { compileMs: number; prepareMs: number; renderMs: number };
type Sample = {
  rows: number;
  backend: string;
  run: number;
  wallMs: number;
  expressions: number;
  templates: number;
  instanceRows: number;
  timings: CheckReport["timings"];
  render: { wallMs: number; compileMs: number; deliveryPrepareMs: number; renderMs: number };
};

function parseArgs(args: string[]) {
  const cli = resolve(option(args, "--cli", join(ROOT, "target/debug/valle")));
  const backend = option(args, "--backend", "raster");
  const repeats = nonnegativeInteger(option(args, "--repeats", "3"), "--repeats", 1);
  const warmups = nonnegativeInteger(option(args, "--warmups", "1"), "--warmups");
  const report = args.includes("--report") ? resolve(option(args, "--report")) : undefined;
  const enforce = args.includes("--enforce");
  if (!(["raster", "metal"] as string[]).includes(backend)) throw new Error("--backend must be raster or metal");
  if (enforce && !report) throw new Error("--enforce requires --report");
  return { cli, backend, repeats, warmups, report, enforce };
}

function countsFromArgs(args: string[]): number[] {
  const index = args.indexOf("--counts");
  if (index < 0) return [10000, 99856];
  const values: number[] = [];
  for (const value of args.slice(index + 1)) {
    if (value.startsWith("--")) break;
    values.push(nonnegativeInteger(value, "--counts", 1));
  }
  if (!values.length || new Set(values).size !== values.length) throw new Error("--counts must contain unique square sizes");
  for (const value of values) {
    if (value > 100000 || Math.sqrt(value) % 1 !== 0) {
      throw new Error("--counts entries must be squares at or below 100000");
    }
  }
  return values;
}

function machine() {
  let cpu = os.cpus()[0]?.model ?? "unknown";
  let model: string | null = null;
  if (os.platform() === "darwin") {
    try { cpu = run(["sysctl", "-n", "machdep.cpu.brand_string"]).trim() || cpu; } catch {
      try {
        const hardware = run(["system_profiler", "SPHardwareDataType"]);
        cpu = hardware.match(/^\s*Chip:\s*(.+)$/m)?.[1] ?? cpu;
        model = hardware.match(/^\s*Model Identifier:\s*(.+)$/m)?.[1] ?? null;
      } catch { /* The OS details are diagnostic only. */ }
    }
  }
  return { system: os.platform(), release: os.release(), architecture: os.arch(), cpu, model };
}

function summarize(samples: Sample[], cli: string) {
  const grouped = new Map<number, Sample[]>();
  for (const sample of samples) grouped.set(sample.rows, [...(grouped.get(sample.rows) ?? []), sample]);
  const medians: Record<string, {
    expressions: number; templates: number; instanceRows: number;
    templateCompileMs: number; instanceDataMs: number; instanceDataMsPer1000: number;
    framePrepareMs: number; renderMs: number;
  }> = {};
  for (const [count, rows] of [...grouped].sort(([a], [b]) => a - b)) {
    medians[String(count)] = {
      expressions: rows[0]!.expressions,
      templates: rows[0]!.templates,
      instanceRows: rows[0]!.instanceRows,
      templateCompileMs: median(rows.map(row => row.timings.templateCompile)),
      instanceDataMs: median(rows.map(row => row.timings.instanceData)),
      instanceDataMsPer1000: median(rows.map(row => row.timings.instanceData)) * 1000 / count,
      framePrepareMs: median(rows.map(row => row.timings.framePrepare)),
      renderMs: median(rows.map(row => row.render.renderMs)),
    };
  }
  const gates: Record<string, { passed: boolean; [key: string]: unknown }> = {};
  if (grouped.has(64) && grouped.has(4096)) {
    const small = medians["64"]!;
    const large = medians["4096"]!;
    const ratio = small.templateCompileMs > 0 ? large.templateCompileMs / small.templateCompileMs : null;
    gates.b01Functional = {
      passed: large.expressions <= small.expressions * 2
        && [64, 4096].every(count => medians[String(count)]?.templates === 1
          && medians[String(count)]?.instanceRows === count),
      expressions64: small.expressions,
      expressions4096: large.expressions,
    };
    gates.templateCompileRatio = { passed: ratio !== null && ratio <= 1.5, actual: ratio, maximum: 1.5 };
    gates.instanceDataMsPer1000 = {
      passed: Object.values(medians).every(row => row.instanceDataMsPer1000 <= 20),
      actualByRows: Object.fromEntries(Object.entries(medians).map(([count, row]) => [count, row.instanceDataMsPer1000])),
      maximum: 20,
    };
  }
  return {
    capturedAtUtc: new Date().toISOString(), machine: machine(), cli: basename(cli), cliSha256: sha256(cli),
    fixture: "crates/valle-compiler/tests/fixtures/motion/composition/grid-instances.motion.tsx",
    repeats: grouped.values().next().value?.length ?? 0,
    medians, gates, passed: Object.keys(gates).length > 0 && Object.values(gates).every(gate => gate.passed),
    samples,
  };
}

function main() {
  const args = Bun.argv.slice(2);
  const { cli, backend, repeats, warmups, report, enforce } = parseArgs(args);
  const counts = countsFromArgs(args);
  const source = readFileSync(fixture, "utf8");
  if (!source.includes("const COLS = 8;")) throw new Error("grid fixture no longer has the expected column count");
  const samples: Sample[] = [];
  const temporary = mkdtempSync(join(tmpdir(), "valle-b01-motion-"));
  try {
    for (const count of counts) {
      const cols = Math.sqrt(count);
      const path = join(temporary, `grid-instances-${cols}.motion.tsx`);
      writeFileSync(path, source.replace("const COLS = 8;", `const COLS = ${cols};`));
      for (let warmup = 0; warmup < warmups; warmup++) {
        run([cli, "motion", "check", path, "--frame", "30", "--json"]);
      }
      for (let index = 0; index < repeats; index++) {
        const started = performance.now();
        const checked = runJson<CheckReport>([cli, "motion", "check", path, "--frame", "30", "--json"]);
        const wallMs = performance.now() - started;
        const output = join(temporary, `grid-instances-${cols}-run-${index + 1}.png`);
        const renderStarted = performance.now();
        const rendered = runJson<{ delivery: { timing: RenderTiming } }>([
          cli, "motion", "render", path, "--frame", "30", "--backend", backend,
          "--output", output, "--json",
        ]);
        const renderWallMs = performance.now() - renderStarted;
        const sample: Sample = {
          rows: count, backend, run: index + 1, wallMs,
          expressions: checked.expressions, templates: checked.templates,
          instanceRows: checked.instanceRows, timings: checked.timings,
          render: { wallMs: renderWallMs, compileMs: rendered.delivery.timing.compileMs,
            deliveryPrepareMs: rendered.delivery.timing.prepareMs,
            renderMs: rendered.delivery.timing.renderMs },
        };
        samples.push(sample);
        console.log(JSON.stringify(sample));
      }
    }
  } finally { rmSync(temporary, { recursive: true, force: true }); }
  if (report) {
    const summary = summarize(samples, cli);
    mkdirSync(dirname(report), { recursive: true });
    writeFileSync(report, JSON.stringify(summary, null, 2) + "\n");
    console.log(JSON.stringify({ report, gates: summary.gates, passed: summary.passed }));
    if (enforce && !summary.passed) process.exitCode = 1;
  }
}

if (import.meta.main) main();
