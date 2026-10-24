/** Compare corresponding captured pixels within each physical backend, independently of cross-backend goldens. */
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { option } from "../../scripts/motion/common.ts";
import { readPng } from "../parity/lib/png.ts";
import { compareFrames, crop } from "./pixels.ts";

const args = Bun.argv.slice(2);
if (args.includes("--help")) {
  console.log(
    "bun web/tests/performance/compare-qa.ts --before RUN_DIR --after RUN_DIR --output NEW_DIR",
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
if (JSON.stringify(before.settings.qa) !== JSON.stringify(after.settings.qa))
  throw new Error("QA resolution/schedule differ");
const qa = after.settings.qa;
const rule = { rgb: 0, alpha: 0 };
const report: any = {
  status: "running",
  before: beforeDir,
  after: afterDir,
  settings: qa,
  rule,
  scope:
    "same backend and captured request: exact premultiplied RGB, alpha and black/white composition; does not waive cross-backend failures",
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
) {
  const { diff, ...result } = compareFrames(reference, actual, rule);
  report.rows.push({ case: c, backend, label, frame, ...result });
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
  if (!before.qa?.length || !after.qa?.length) throw new Error("both runs need captured QA rows");
  for (const { case: c, backend } of before.qa) {
    if (!after.qa.some((row: any) => row.case === c && row.backend === backend))
      throw new Error(`missing current case/backend: ${c}/${backend}`);
    if (JSON.stringify(before.identities.cases[c]) !== JSON.stringify(after.identities.cases[c]))
      throw new Error(`fixture identity differs: ${c}`);
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
          );
        }
    }
    const rows = report.rows.slice(start);
    console.log(
      `${backend}/${c}: ${rows.filter((row: any) => !row.pass).length}/${rows.length} changed captures`,
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
