/** Measure B03 frame preparation and evaluated expression work. */
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { nonnegativeInteger, option, ROOT, runJson } from "./common.ts";

type CheckReport = {
  nodes: number;
  templates: number;
  instanceRows: number;
  expressions: number;
  evaluatedExpressions: number;
  fullEvaluationExpressions: number;
  timings: { framePrepare: number };
};

function main() {
  const args = Bun.argv.slice(2);
  const cli = resolve(option(args, "--cli", join(ROOT, "target/debug/valle")));
  const repeats = nonnegativeInteger(option(args, "--repeats", "3"), "--repeats", 1);
  const fixture = join(ROOT, "crates/valle-compiler/tests/fixtures/motion/composition/lazy-evaluation.motion.tsx");
  const source = readFileSync(fixture, "utf8");
  if (!source.includes("length: 2000")) throw new Error("B03 fixture no longer contains 2000 rows");
  const temporary = mkdtempSync(join(tmpdir(), "valle-b03-motion-"));
  try {
    for (const rows of [2000, 20000]) {
      const path = join(temporary, `lazy-evaluation-${rows}.motion.tsx`);
      writeFileSync(path, source.replace("length: 2000", `length: ${rows}`));
      for (let index = 0; index < repeats; index++) {
        const checked = runJson<CheckReport>([cli, "motion", "check", path, "--frame", "1", "--json"]);
        console.log(JSON.stringify({
          rows, run: index + 1, nodes: checked.nodes, templates: checked.templates,
          instanceRows: checked.instanceRows, expressions: checked.expressions,
          evaluatedExpressions: checked.evaluatedExpressions,
          fullEvaluationExpressions: checked.fullEvaluationExpressions,
          framePrepareMs: checked.timings.framePrepare,
        }));
      }
    }
  } finally { rmSync(temporary, { recursive: true, force: true }); }
}

if (import.meta.main) main();
