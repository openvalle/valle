// Record official D3 results for review; never overwrite accepted baselines.
import { createHash } from "node:crypto";
import { join } from "node:path";
import * as d3 from "d3";

const repo = join(import.meta.dir, "../..");
const source = join(repo, "tests/cases/d3/cases.ts");
Object.assign(globalThis, d3);
const { cases } = (await import(source)) as { cases: Record<string, () => unknown> };
const results: Record<string, unknown> = {};
for (const [name, run] of Object.entries(cases)) results[name] = run();
const version: string = (await Bun.file(Bun.resolveSync("d3/package.json", import.meta.dir)).json())
  .version;
const output = join(repo, "target/baseline-candidates/d3/results.json");
await Bun.write(
  output,
  JSON.stringify(
    {
      d3: version,
      source_sha256: createHash("sha256")
        .update(await Bun.file(source).bytes())
        .digest("hex"),
      results,
    },
    null,
    2,
  ) + "\n",
);
console.log(`Candidate official results: ${output}`);
