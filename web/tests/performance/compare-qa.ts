/** Compare corresponding captured pixels within each physical backend, independently of cross-backend goldens. */
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { option } from "../../scripts/motion/common.ts";
import { readPng } from "../parity/lib/png.ts";
import { compareFrames, crop, type PixelRule } from "./pixels.ts";
import { comparisonCases } from "./qa-comparison.ts";

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
  for (const { case: c, backend, rule } of comparisonCases(before, after, rules)) {
    const beforeFolder = path.join(beforeDir, "qa", backend, c);
    const afterFolder = path.join(afterDir, "qa", backend, c);
    const start = report.rows.length;
    if (backend.startsWith("native")) {
      for (const threads of qa.threadAllowances) {
        const reference = await readPng(path.join(beforeFolder, `threads-${threads}`, "sheet.png"));
        const actual = await readPng(path.join(afterFolder, `threads-${threads}`, "sheet.png"));
        const columns = Math.min(4, qa.order.length);
        for (const [sequence, frame] of qa.order.entries()) {
          const x = (sequence % columns) * qa.width,
            y = Math.floor(sequence / columns) * qa.height;
          await compare(
            crop(reference, x, y, qa.width, qa.height),
            crop(actual, x, y, qa.width, qa.height),
            c,
            backend,
            `n${threads}-${sequence}`,
            frame,
            rule,
          );
        }
      }
    } else {
      for (const session of ["warm", "bypass"])
        for (const [sequence, frame] of qa.order.entries()) {
          const label = `${session}-${sequence}-${frame}`;
          await compare(
            await readPng(path.join(beforeFolder, `${label}.png`)),
            await readPng(path.join(afterFolder, `${label}.png`)),
            c,
            backend,
            label,
            frame,
            rule,
          );
        }
    }
    const rows = report.rows.slice(start);
    console.log(
      `${backend}/${c}: ${rows.filter((row: any) => !row.pass).length}/${rows.length} captures outside tolerance`,
    );
    await save();
  }
  report.status = report.rows.every((row: any) => row.pass) ? "passed" : "failed";
  report.comparisons = report.rows.length;
} catch (error) {
  report.status = "failed";
  report.error = String(error);
} finally {
  await save();
}
if (report.status !== "passed") process.exitCode = 1;
