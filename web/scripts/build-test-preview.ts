// Playback-only fixture, deliberately outside the public runtime manifest and distribution.
import path from "node:path";
import { withBuildDirectory } from "./build-directory.ts";
const root = path.resolve(import.meta.dir, "..");
await withBuildDirectory(path.join(root, "test-dist", "preview"), async (directory) => {
  const result = await Bun.build({
    entrypoints: [path.join(root, "tests/preview/index.html")],
    root: path.join(root, "tests/preview"), outdir: directory,
    target: "browser", format: "esm", splitting: false,
  });
  if (!result.success) throw new Error(result.logs.map((log) => log.message).join("\n"));
});
