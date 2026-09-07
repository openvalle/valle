import { expect, test } from "bun:test";
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { withBuildDirectory } from "./build-directory.ts";

test("failed builds preserve the last runtime and successful builds replace obsolete files", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "valle-build-"));
  const output = path.join(root, "dist");
  try {
    await mkdir(output);
    await writeFile(path.join(output, "old.js"), "working runtime");
    await expect(withBuildDirectory(output, async (next) => {
      await writeFile(path.join(next, "partial.js"), "incomplete");
      throw new Error("compiler failed");
    })).rejects.toThrow("compiler failed");
    expect(await readFile(path.join(output, "old.js"), "utf8")).toBe("working runtime");
    expect(await readdir(root)).toEqual(["dist"]);
    await withBuildDirectory(output, async (next) => {
      await writeFile(path.join(next, "new.js"), "complete");
    });
    expect(await readdir(output)).toEqual(["new.js"]);
    expect(await readFile(path.join(output, "new.js"), "utf8")).toBe("complete");
    expect(await readdir(root)).toEqual(["dist"]);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
