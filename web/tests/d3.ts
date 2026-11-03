// What d3 of package.json gives for the checks of tests/cases/d3/checks.js, for the case runner,
// which keeps it as tests/baselines/d3/checks.json and holds the built-in charts to it.
//
//   bun web/tests/d3.ts <checks.js> <out.json>
//
// The checks call d3's functions as globals, as scenes call the built-ins.

import { resolve } from "node:path";
import * as d3 from "d3";

const [checks, out] = process.argv.slice(2);
Object.assign(globalThis, d3);
const module = (await import(resolve(checks))) as { checks: Record<string, () => unknown> };
const results: Record<string, unknown> = {};
for (const [name, check] of Object.entries(module.checks)) results[name] = check();
const version: string = (await Bun.file(Bun.resolveSync("d3/package.json", import.meta.dir)).json())
  .version;
await Bun.write(out, JSON.stringify({ d3: version, results }));
