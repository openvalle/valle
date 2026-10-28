/** Compare corresponding captured pixels within each physical backend, independently of cross-backend goldens. */
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { option } from "../../scripts/motion/common.ts";
import { readPng } from "../parity/lib/png.ts";
import { compareFrames, crop, type PixelRule } from "./pixels.ts";
import { comparisonCases } from "./qa-comparison.ts";
import { compareSequence, type TemporalFrame } from "./temporal.ts";

const args = Bun.argv.slice(2);
if (args.includes("--help")) {
  console.log(
    "bun web/tests/performance/compare-qa.ts --before RUN_DIR --after RUN_DIR --output NEW_DIR [--rules exact|registered]",
  );
  process.exit(0);
}
const beforeDir = path.resolve(option(args, "--before"));
const afterDir = path.resolve(option(args, "--after"));
const output = path.resolve(option(args, "--output"));
if (await Bun.file(path.join(output, "run.json")).exists())
  throw new Error("comparison directory already used");
const before = await Bun.file(path.join(beforeDir, "run.json")).json();
const after = await Bun.file(path.join(afterDir, "run.json")).json();
const qa = after.settings.qa;
const rules = option(args, "--rules", "registered");
if (rules !== "exact" && rules !== "registered") throw new Error("invalid comparison rules");
const report: any = {
  status: "running",
  before: beforeDir,
  after: afterDir,
  settings: qa,
  rules,
  scope:
    "same backend and captured request: premultiplied RGB, alpha and black/white composition; repeat and cross-backend gates remain independent",
  rows: [],
  temporal: [],
};
await mkdir(output, { recursive: true });
const save = () => writeFile(path.join(output, "run.json"), JSON.stringify(report, null, 2) + "\n");
await save();

async function compare(
  reference: Awaited<ReturnType<typeof readPng>>,
  actual: Awaited<ReturnType<typeof readPng>>,
  c: string,
  backend: string,
  label: string,
  frame: number,
  rule: PixelRule,
) {
  const { diff, ...result } = compareFrames(reference, actual, rule);
  report.rows.push({ case: c, backend, label, frame, rule, ...result });
  if (!result.pass) {
    const folder = path.join(output, backend, c);
    await mkdir(folder, { recursive: true });
    const child = Bun.spawn(
      [
        "ffmpeg",
        "-v",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        `${qa.width}x${qa.height}`,
        "-i",
        "pipe:0",
        "-frames:v",
        "1",
        path.join(folder, `${label}.diff.png`),
      ],
      { stdin: diff.data, stdout: "ignore", stderr: "pipe" },
    );
    const stderr = await new Response(child.stderr).text();
    if (await child.exited) throw new Error(`diff encoding failed: ${stderr}`);
  }
}

try {
  for (const { case: c, backend, rule, temporal } of comparisonCases(before, after, rules)) {
    const beforeFolder = path.join(beforeDir, "qa", backend, c);
    const afterFolder = path.join(afterDir, "qa", backend, c);
    const start = report.rows.length;
    const continuous = new Map<string, TemporalFrame[]>();
    const capture = async (
      reference: TemporalFrame["reference"],
      actual: TemporalFrame["actual"],
      session: string,
      sequence: number,
      frame: number,
      label: string,
    ) => {
      await compare(reference, actual, c, backend, label, frame, rule);
      if (temporal && sequence < qa.continuousFrames) {
        if (!continuous.has(session)) continuous.set(session, []);
        continuous.get(session)!.push({ frame, reference, actual });
      }
    };
    if (backend.startsWith("native")) {
      for (const threads of qa.threadAllowances) {
        const reference = await readPng(path.join(beforeFolder, `threads-${threads}`, "sheet.png"));
        const actual = await readPng(path.join(afterFolder, `threads-${threads}`, "sheet.png"));
        const columns = Math.min(4, qa.order.length);
        for (const [sequence, frame] of qa.order.entries()) {
          const x = (sequence % columns) * qa.width,
            y = Math.floor(sequence / columns) * qa.height;
          await capture(
            crop(reference, x, y, qa.width, qa.height),
            crop(actual, x, y, qa.width, qa.height),
            `n${threads}`,
            sequence,
            frame,
            `n${threads}-${sequence}`,
          );
        }
      }
    } else {
      for (const session of ["warm", "bypass"])
        for (const [sequence, frame] of qa.order.entries()) {
          const label = `${session}-${sequence}-${frame}`;
          await capture(
            await readPng(path.join(beforeFolder, `${label}.png`)),
            await readPng(path.join(afterFolder, `${label}.png`)),
            session,
            sequence,
            frame,
            label,
          );
        }
    }
    if (temporal) {
      if (continuous.size !== (backend.startsWith("native") ? qa.threadAllowances.length : 2))
        throw new Error("missing continuous comparison sessions");
      for (const [session, frames] of continuous) {
        if (frames.length !== qa.continuousFrames)
          throw new Error(`incomplete continuous comparison: ${session}`);
        report.temporal.push({
          case: c,
          backend,
          session,
          ...compareSequence(frames, temporal.rule, temporal.tracks),
        });
      }
    }
    const rows = report.rows.slice(start);
    console.log(
      `${backend}/${c}: ${rows.filter((row: any) => !row.pass).length}/${rows.length} captures outside tolerance`,
    );
    await save();
  }
  report.status =
    report.rows.every((row: any) => row.pass) && report.temporal.every((row: any) => row.pass)
      ? "passed"
      : "failed";
  report.comparisons = report.rows.length;
} catch (error) {
  report.status = "failed";
  report.error = String(error);
} finally {
  await save();
}
if (report.status !== "passed") process.exitCode = 1;
