export const APP_BUILDS = [
  ["preview", "apps/preview/index.html"],
  ["studio", "apps/studio/index.html"],
  ["console", "apps/console/index.html"],
] as const;

export const PACKAGE_BUILDS = [
  ["engine", [
    "packages/engine/src/index.ts",
    "packages/engine/src/element.ts",
    "packages/engine/src/compiler.ts",
  ]],
] as const;

export const WORKER_BUILDS = [
  ["product-frame", "packages/engine/src/runtime/compositor/product-frame-worker.ts"],
  ["motion-curves", "apps/studio/src/motion-curves-worker.ts"],
] as const;

export function assertRuntimeEntrypoints(
  builds: ReadonlyArray<readonly [string, readonly string[]]> = PACKAGE_BUILDS,
): void {
  const entries = builds.flatMap(([, values]) => values);
  if (new Set(entries).size !== entries.length) throw new Error("runtime entrypoints must be unique");
  const workerEntries = WORKER_BUILDS.map(([, entry]) => entry);
  if (new Set(workerEntries).size !== workerEntries.length) throw new Error("worker entrypoints must be unique");
}
