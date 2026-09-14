import { describe, expect, test } from "bun:test";
import { readdir } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const dist = path.resolve(import.meta.dir, "../packages/engine/dist");
const manifest = await Bun.file(path.join(dist, "package.json")).json();

describe("distributed engine SDK", () => {
  test("loads playback and compiler APIs without a DOM or the optional element", async () => {
    const entry = pathToFileURL(path.join(dist, "index.mjs")).href;
    const script = `
      for (const name of ["HTMLElement", "customElements", "document"]) {
        Object.defineProperty(globalThis, name, { get() { throw new Error("UI access: " + name); } });
      }
      const sdk = await import(${JSON.stringify(entry)});
      if (typeof sdk.createBrowserValleWebPlayer !== "function"
        || typeof sdk.createTimelineCompilerRuntime !== "function"
        || typeof sdk.VallePlayerController !== "function"
        || "VallePlayerElement" in sdk) throw new Error("incomplete or UI-coupled engine entry");
    `;
    const child = Bun.spawn([process.execPath, "-e", script], { stdout: "pipe", stderr: "pipe" });
    const error = await new Response(child.stderr).text();
    expect(await child.exited, error).toBe(0);
  });

  test("ships its own WASM and worker, complete declarations, and external CanvasKit", async () => {
    expect(manifest.name).toBe("valle-engine");
    expect(manifest.private).not.toBe(true);
    expect(manifest.dependencies["canvaskit-wasm"]).toMatch(/^\d+\.\d+\.\d+$/);
    expect(manifest.peerDependenciesMeta.lit.optional).toBe(true);
    const files = await listFiles(dist);
    expect(files.filter((file) => file.endsWith(".wasm"))).toEqual(["wasm/valle_engine_bg.wasm"]);
    expect(files.some((file) => /\.(ttf|otf|woff2?)$/.test(file))).toBe(false);
    expect(files).toContain("workers/product-frame.js");
    expect(files).toContain("element.mjs");
    expect(files).toContain("NOTICE.txt");
    for (const entry of Object.values(manifest.exports) as unknown[]) {
      if (entry && typeof entry === "object" && "types" in entry) {
        expect(await Bun.file(path.join(dist, String(entry.types))).exists()).toBe(true);
      }
    }
    // A consumer must not need the repository's Rust schema directory to resolve SDK types.
    for (const file of files.filter((file) => file.endsWith(".d.ts"))) {
      const source = await Bun.file(path.join(dist, file)).text();
      for (const match of source.matchAll(/["'](\.[^"']+)\.js["']/g)) {
        const declaration = path.resolve(dist, path.dirname(file), `${match[1]}.d.ts`);
        expect(declaration.startsWith(`${dist}${path.sep}`), declaration).toBe(true);
        expect(await Bun.file(declaration).exists(), declaration).toBe(true);
      }
    }
  });
});

async function listFiles(dir: string, base = dir): Promise<string[]> {
  const files: string[] = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) files.push(...await listFiles(file, base));
    else files.push(path.relative(base, file).replaceAll(path.sep, "/"));
  }
  return files.sort();
}
