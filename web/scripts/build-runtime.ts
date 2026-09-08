import { withBuildDirectory } from "./build-directory.ts";
import { mkdir, readdir } from "node:fs/promises";
import path from "node:path";

import {
  assertRuntimeEntrypoints,
  APP_BUILDS,
  PACKAGE_BUILDS,
  WORKER_BUILDS,
} from "./runtime-entrypoints.ts";
import { emitRuntimeManifests } from "./runtime-manifest.ts";
import {
  assertCanvasKitBuildVersion,
  assertGeneratedEngineBuildVersion,
  readRuntimeProtocolVersion,
  readValleWorkspaceVersion,
} from "./runtime-version.ts";

const root = path.resolve(import.meta.dir, "..");
await withBuildDirectory(path.join(root, "dist"), async (dist) => {
  const valleBuildVersion = await readValleWorkspaceVersion(path.resolve(root, ".."));
  const runtimeProtocolVersion = await readRuntimeProtocolVersion(path.resolve(root, ".."));

  assertRuntimeEntrypoints();
  for (const [name, entrypoints] of PACKAGE_BUILDS) {
    const result = await Bun.build({
      entrypoints: entrypoints.map((entry) => path.join(root, entry)),
      outdir: path.join(dist, "packages", name),
      format: "esm",
      target: "browser",
      sourcemap: "external",
      naming: { entry: "[dir]/[name].mjs" },
    });
    assertBuild(result, `package ${name}`);
  }

  const workerResult = await Bun.build({
    entrypoints: WORKER_BUILDS.map(([, entrypoint]) => path.join(root, entrypoint)),
    outdir: path.join(dist, "runtime", "workers"),
    format: "esm",
    target: "browser",
    sourcemap: "external",
    naming: { entry: "product-frame.js" },
  });
  assertBuild(workerResult, "Product frame worker");

  const appResult = await Bun.build({
    entrypoints: APP_BUILDS.map(([, entrypoint]) => path.join(root, entrypoint)),
    outdir: dist,
    root,
    target: "browser",
    format: "esm",
    splitting: true,
    naming: { chunk: "chunks/[name]-[hash].[ext]" },
    sourcemap: "external",
  });
  assertBuild(appResult, "apps");

  const generatedEngine = path.join(root, "packages", "engine", "generated", "web");
  await assertGeneratedEngineBuildVersion(generatedEngine, valleBuildVersion);
  const engineRuntime = path.join(dist, "runtime", "engine");
  await mkdir(engineRuntime, { recursive: true });
  for (const asset of [
    "valle_engine.js",
    "valle_engine_bg.wasm",
    "valle_engine.d.ts",
  ] as const) {
    await Bun.write(path.join(engineRuntime, asset), Bun.file(path.join(generatedEngine, asset)));
  }

  const playerCoreRoot = path.join(root, "packages", "player-core");
  const canvasKitRoot = path.dirname(path.dirname(Bun.resolveSync("canvaskit-wasm", playerCoreRoot)));
  await assertCanvasKitBuildVersion(canvasKitRoot);
  const mp4boxRoot = path.dirname(path.dirname(Bun.resolveSync("mp4box", playerCoreRoot)));
  for (const [source, target] of [
    [path.join(canvasKitRoot, "bin", "canvaskit.js"), "runtime/canvaskit/base/canvaskit.js"],
    [path.join(canvasKitRoot, "bin", "canvaskit.wasm"), "runtime/canvaskit/base/canvaskit.wasm"],
    [path.join(canvasKitRoot, "bin", "full", "canvaskit.js"), "runtime/canvaskit/full/canvaskit.js"],
    [path.join(canvasKitRoot, "bin", "full", "canvaskit.wasm"), "runtime/canvaskit/full/canvaskit.wasm"],
    [path.join(canvasKitRoot, "LICENSE"), "runtime/licenses/canvaskit.txt"],
    [path.join(mp4boxRoot, "LICENSE"), "runtime/licenses/mp4box.txt"],
    [path.resolve(root, "..", "assets", "fonts", "noto", "NotoSans-Regular.ttf"), "runtime/fonts/NotoSans-Regular.ttf"],
  ] as const) {
    const destination = path.join(dist, target);
    await mkdir(path.dirname(destination), { recursive: true });
    await Bun.write(destination, Bun.file(source));
  }

  // Preserve notices for fonts and adapted code embedded in the runtime.
  const licenseRoots = [
    "assets/fonts",
    "crates/valle-motion/licenses",
    "crates/valle-draw/licenses",
  ];
  const repo = path.resolve(root, "..");
  const notices = [await Bun.file(path.join(repo, "LICENSE")).text()];
  for (const relative of licenseRoots) {
    const files = (await listFiles(path.join(repo, relative))).filter((file) => file.endsWith(".txt")).sort();
    for (const file of files) {
      notices.push(`${path.relative(repo, file)}\n\n${await Bun.file(file).text()}`);
    }
  }
  await Bun.write(path.join(dist, "runtime", "licenses", "valle-and-third-party.txt"), notices.join("\n\n"));

  await emitRuntimeManifests(dist, valleBuildVersion, runtimeProtocolVersion, {
    canvasKit: (await Bun.file(path.join(canvasKitRoot, "package.json")).json()).version,
    mp4box: (await Bun.file(path.join(mp4boxRoot, "package.json")).json()).version,
  });

  const outputs = await listFiles(dist);
  console.log(`built ${outputs.length} files under dist`);
});

function assertBuild(result: Awaited<ReturnType<typeof Bun.build>>, label: string): void {
  if (!result.success) {
    const messages = result.logs.map((log) => log.message).join("\n");
    throw new Error(`${label} build failed:\n${messages}`);
  }
}

async function listFiles(dir: string): Promise<string[]> {
  const files: string[] = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const absolute = path.join(dir, entry.name);
    if (entry.isDirectory()) files.push(...await listFiles(absolute));
    else files.push(absolute);
  }
  return files;
}
