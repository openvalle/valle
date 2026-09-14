import { cp, readdir, rm } from "node:fs/promises";
import path from "node:path";
import { withBuildDirectory } from "./build-directory.ts";
import { assertCanvasKitBuildVersion, assertGeneratedEngineBuildVersion, readValleWorkspaceVersion } from "./runtime-version.ts";
import { thirdPartyNotices } from "./third-party-notices.ts";
import workspace from "../package.json";
import sdk from "../packages/engine/package.json";

const web = path.resolve(import.meta.dir, "..");
const repo = path.resolve(web, "..");
const root = path.join(web, "packages/engine");
const generated = path.join(root, "generated/web");
const version = await readValleWorkspaceVersion(repo);
await assertGeneratedEngineBuildVersion(generated, version);
await assertCanvasKitBuildVersion(path.dirname(path.dirname(Bun.resolveSync("canvaskit-wasm", root))));

await withBuildDirectory(path.join(root, "dist"), async (dist) => {
  const entries = Object.values(sdk.exports);
  const output = await Bun.build({
    entrypoints: entries.map((entry) => path.join(root, entry)),
    root: path.join(root, "src"),
    outdir: dist,
    target: "browser",
    format: "esm",
    splitting: true,
    external: ["lit", "mp4box"],
    naming: { entry: "[dir]/[name].mjs", chunk: "chunks/[name]-[hash].mjs" },
  });
  assertBuild(output, "SDK");
  assertBuild(await Bun.build({
    entrypoints: [path.join(root, "src/runtime/compositor/product-frame-worker.ts")],
    outdir: path.join(dist, "workers"),
    target: "browser",
    format: "esm",
    naming: "product-frame.js",
  }), "SDK worker");

  const types = path.join(dist, "types");
  const declarations = path.join(dist, ".declarations");
  const tsc = Bun.spawn([
    process.execPath, "run", "tsc", "--project", path.join(root, "tsconfig.build.json"), "--outDir", declarations,
  ], { cwd: web, stdout: "inherit", stderr: "inherit" });
  if (await tsc.exited !== 0) throw new Error("SDK declaration generation failed");
  await cp(path.join(declarations, "web/packages/engine/src"), types, { recursive: true });
  await cp(path.join(declarations, "crates/valle-timeline/schema"), path.join(types, "schema"), { recursive: true });
  await rm(declarations, { recursive: true });
  await rewriteDeclarationImports(types);

  // Keep the generated glue and its WASM together, including any bindgen snippets.
  await cp(generated, path.join(dist, "wasm"), {
    recursive: true,
    filter: (source) => !["package.json", "LICENSE"].includes(path.basename(source)),
  });
  await Bun.write(path.join(dist, "LICENSE"), Bun.file(path.join(repo, "LICENSE")));
  await Bun.write(path.join(dist, "NOTICE.txt"), await thirdPartyNotices(repo));
  await Bun.write(path.join(dist, "README.md"), Bun.file(path.join(root, "README.md")));

  const exports = Object.fromEntries(Object.entries(sdk.exports).map(([name, entry]) => {
    const relative = entry.replace(/^\.\/src\//, "").replace(/\.ts$/, "");
    return [name, { types: `./types/${relative}.d.ts`, import: `./${relative}.mjs`, default: `./${relative}.mjs` }];
  }));
  await Bun.write(path.join(dist, "package.json"), JSON.stringify({
    name: sdk.name,
    version,
    description: "Programmable video engine for the browser, with Motion playback and Timeline compilation.",
    license: "Apache-2.0",
    repository: { type: "git", url: "https://github.com/openvalle/valle", directory: "web/packages/engine" },
    type: "module",
    main: "./index.mjs",
    module: "./index.mjs",
    types: "./types/index.d.ts",
    exports: { ...exports, "./wasm/*": "./wasm/*", "./workers/*": "./workers/*", "./package.json": "./package.json" },
    files: ["**/*.mjs", "types", "wasm", "workers", "LICENSE", "NOTICE.txt", "README.md"],
    sideEffects: ["./element.mjs", "./wasm/snippets/**"],
    dependencies: { ...sdk.dependencies, "canvaskit-wasm": workspace.workspaces.catalog["canvaskit-wasm"] },
    peerDependencies: sdk.peerDependencies,
    peerDependenciesMeta: sdk.peerDependenciesMeta,
  }, null, 2) + "\n");
});
console.log("built npm SDK under packages/engine/dist (CanvasKit remains an official dependency)");

function assertBuild(result: Awaited<ReturnType<typeof Bun.build>>, label: string): void {
  if (!result.success) throw new Error(`${label}: ${result.logs.map((log) => log.message).join("\n")}`);
}

async function rewriteDeclarationImports(dir: string): Promise<void> {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) await rewriteDeclarationImports(file);
    else if (file.endsWith(".d.ts")) {
      const source = await Bun.file(file).text();
      await Bun.write(file, source
        .replaceAll("../../../../crates/valle-timeline/schema/", "./schema/")
        .replace(/(["']\.[^"']+)\.ts(["'])/g, "$1.js$2"));
    }
  }
}
