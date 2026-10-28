/** Inspect real, pre-delivery F16 captures; keep cross-backend and exact repeat gates distinct. */
import path from "node:path";
import { option } from "../../scripts/motion/common.ts";
import { compareWorkingF16, inspectWorkingF16 } from "./working-f16.ts";
const run = path.resolve(option(Bun.argv.slice(2), "--run"));
const report = await Bun.file(path.join(run, "run.json")).json();
const suite = await Bun.file(path.join(import.meta.dir, "suite.json")).json();
if (!report.settings.workingF16) throw new Error("run did not request pre-delivery F16 captures");
const order: number[] = report.settings.qa.order;
const bytes = report.settings.qa.width * report.settings.qa.height * 8;
const rows: unknown[] = [];
let passed = true;
for (const id of Object.keys(report.identities.cases)) {
  const registered = suite.cases.find((c: { id: string }) => c.id === id);
  if (!registered?.workingRule) throw new Error(`no registered F16 rule for ${id}`);
  const rule = report.settings.workingRules?.[id];
  if (!rule || JSON.stringify(rule) !== JSON.stringify(registered.workingRule))
    throw new Error(`F16 contract missing or changed after capture: ${id}`);
  const nativeReferences = new Map<number, Uint8Array>();
  for (const backend of report.settings.backends as string[]) {
    const folder = path.join(run, "qa", backend, id);
    const captured: { frame: number; sequence: number; file: string; session: string }[] = [];
    if (backend.startsWith("native")) {
      for (const threads of report.settings.qa.threadAllowances as number[]) {
        const session = `threads-${threads}`;
        const result = await Bun.file(path.join(folder, session, "stdout.json")).json();
        if (result.workingCaptures?.length !== order.length)
          throw new Error(`missing F16 captures: ${backend}/${id}/${session}`);
        for (const entry of result.workingCaptures)
          captured.push({ ...entry, file: path.join(session, entry.file), session });
      }
    } else {
      const result = await Bun.file(path.join(folder, "browser.json")).json();
      for (const entry of result.outputs) {
        if (!entry.name) continue;
        if (entry.working?.length !== 1)
          throw new Error(`missing F16 capture: ${backend}/${id}/${entry.name}`);
        captured.push({
          frame: entry.frame,
          sequence: entry.sequence,
          file: entry.working[0].name,
          session: entry.bypass ? "bypass" : "warm",
        });
      }
      if (captured.length !== order.length * 2)
        throw new Error(`incomplete browser F16 coverage: ${backend}/${id}`);
    }
    const repeats = new Map<number, Uint8Array>();
    for (const entry of captured) {
      if (order[entry.sequence] !== entry.frame)
        throw new Error("working capture frame order mismatch");
      const raw = await Bun.file(path.join(folder, entry.file)).bytes();
      if (raw.byteLength !== bytes) throw new Error("working F16 extent mismatch");
      const range = inspectWorkingF16(raw);
      const valid =
        range.nonfinite === 0 && (!rule.requireHdr || (range.hdr > 0 && range.negative > 0));
      if (backend === "native-cpu" && !nativeReferences.has(entry.frame))
        nativeReferences.set(entry.frame, raw);
      const reference = nativeReferences.get(entry.frame);
      if (!reference) throw new Error("Native CPU references must precede other backends");
      const cross = compareWorkingF16(reference, raw, rule);
      const repeated = repeats.get(entry.frame);
      const repeat = repeated ? compareWorkingF16(repeated, raw, { rgb: 0, alpha: 0 }) : null;
      if (!repeated) repeats.set(entry.frame, raw);
      const pass = valid && cross.pass && (!repeat || repeat.differentBits === 0);
      passed &&= pass;
      rows.push({
        case: id,
        backend,
        ...entry,
        rule,
        valid,
        range,
        cross,
        repeat,
        pass,
      });
    }
  }
}
const result = {
  status: passed ? "passed" : "failed",
  format: "linear-rec2020-premul-rgba16f-le",
  stage: "working image before delivery transform",
  rows,
};
await Bun.write(path.join(run, "working-f16.json"), JSON.stringify(result, null, 2) + "\n");
console.log(`${result.status}: ${rows.length} pre-delivery F16 captures`);
if (!passed) process.exitCode = 1;
