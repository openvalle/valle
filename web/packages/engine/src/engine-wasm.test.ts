import { afterAll, expect, test } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { loadEngineWasm } from "./engine-wasm.ts";

const directory = await mkdtemp(join(tmpdir(), "valle-wasm-load-"));
afterAll(() => rm(directory, { recursive: true }));
async function moduleUrl(name: string, source: string): Promise<string> {
  const path = join(directory, `${name}.mjs`);
  await writeFile(path, source);
  return pathToFileURL(path).href;
}

const sharedUrl = await moduleUrl(
  "shared",
  `
    export let calls = 0;
    export let input;
    export default async function(options) { calls++; input=options.module_or_path; }
  `,
);
const retryUrl = await moduleUrl(
  "retry",
  `export let calls = 0; export default async function() { if (++calls === 1) throw new Error("temporary failure"); }`,
);

test("concurrent compiler and player loads share one initialization", async () => {
  const [compiler, player] = (await Promise.all([
    loadEngineWasm(sharedUrl, "engine.wasm"),
    loadEngineWasm(sharedUrl, "engine.wasm"),
  ])) as Array<{ calls: number; input: string }>;
  expect(compiler).toBe(player);
  expect(compiler!.calls).toBe(1);
  expect(compiler!.input).toBe("engine.wasm");
});

test("failed warmup does not prevent a subsequent load", async () => {
  await expect(loadEngineWasm(retryUrl, "retry.wasm")).rejects.toThrow("temporary failure");
  const module = (await loadEngineWasm(retryUrl, "retry.wasm")) as { calls: number };
  expect(module.calls).toBe(2);
});
