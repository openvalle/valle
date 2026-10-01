/** Measure the 1080p F02 bloom frame beside a no-filter control. */
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { nonnegativeInteger, option, ROOT, runJson } from "./common.ts";

const bloomAttribute = " bloom={{ threshold: 0.8, intensity: 1.2, radius: 48 }}";
type RenderReport = { delivery: { timing: { renderMs: number; prepareMs: number; compileMs: number } } };

function main() {
  const args = Bun.argv.slice(2);
  const cli = resolve(option(args, "--cli", join(ROOT, "target/debug/valle")));
  const backend = option(args, "--backend", "raster");
  const repeats = nonnegativeInteger(option(args, "--repeats", "3"), "--repeats", 1);
  if (backend !== "raster" && backend !== "metal") throw new Error("--backend must be raster or metal");
  const fixture = join(ROOT, "crates/valle-compiler/tests/fixtures/motion/composition/scene-bloom.motion.tsx");
  const source = readFileSync(fixture, "utf8").replace("width: 640, height: 360", "width: 1920, height: 1080");
  if (!source.includes(bloomAttribute)) throw new Error("F02 fixture no longer contains the bloom attribute");
  const temporary = mkdtempSync(join(tmpdir(), "valle-f02-motion-"));
  try {
    for (const enabled of [false, true]) {
      const effect = enabled ? "bloom" : "control";
      const path = join(temporary, `${effect}.motion.tsx`);
      writeFileSync(path, enabled ? source : source.replace(bloomAttribute, ""));
      for (let index = 0; index < repeats; index++) {
        const output = join(temporary, `${effect}-${index}.png`);
        const report = runJson<RenderReport>([
          cli, "--json", "motion", "render", path, "--frame", "0",
          "--backend", backend, "--output", output,
        ]);
        console.log(JSON.stringify({
          effect, backend, width: 1920, height: 1080, run: index + 1,
          renderMs: report.delivery.timing.renderMs,
          prepareMs: report.delivery.timing.prepareMs,
          compileMs: report.delivery.timing.compileMs,
        }));
      }
    }
  } finally { rmSync(temporary, { recursive: true, force: true }); }
}

if (import.meta.main) main();
