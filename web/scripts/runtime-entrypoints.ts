export const APP_BUILDS = [
  ["preview", "apps/preview/index.html"],
  ["studio", "apps/studio/index.html"],
  ["console", "apps/console/index.html"],
] as const;

export const PACKAGE_BUILDS = [
  ["engine", ["packages/engine/src/index.ts"]],
  ["player-core", ["packages/player-core/src/index.ts"]],
  ["player", ["packages/player/src/index.ts"]],
] as const;

export const WORKER_BUILDS = [
  ["product-frame", "packages/player-core/src/compositor/product-frame-worker.ts"],
] as const;

export function assertRuntimeEntrypoints(
  builds: ReadonlyArray<readonly [string, readonly string[]]> = PACKAGE_BUILDS,
): void {
  const entries = builds.flatMap(([, values]) => values);
  if (new Set(entries).size !== entries.length) throw new Error("runtime entrypoints must be unique");
  const workerEntries = WORKER_BUILDS.map(([, entry]) => entry);
  if (new Set(workerEntries).size !== workerEntries.length) throw new Error("worker entrypoints must be unique");
}
