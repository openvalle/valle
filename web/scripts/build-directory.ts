import { mkdir, mkdtemp, rename, rm, stat } from "node:fs/promises";
import path from "node:path";

/** Publish only complete builds, preserving the previous output on failure. */
export async function withBuildDirectory(output: string, build: (next: string) => Promise<void>): Promise<void> {
  await mkdir(path.dirname(output), { recursive: true });
  const scratch = await mkdtemp(path.join(path.dirname(output), `.${path.basename(output)}-build-`));
  const next = path.join(scratch, "next");
  const previous = path.join(scratch, "previous");
  let backedUp = false;
  try {
    await mkdir(next);
    await build(next);
    if (await exists(output)) {
      await rename(output, previous);
      backedUp = true;
    }
    try { await rename(next, output); }
    catch (error) {
      if (backedUp) { await rename(previous, output); backedUp = false; }
      throw error;
    }
    await rm(scratch, { recursive: true });
  } catch (error) {
    if (!backedUp) await rm(scratch, { recursive: true, force: true });
    throw error;
  }
}

async function exists(file: string): Promise<boolean> {
  try { await stat(file); return true; }
  catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return false; throw error; }
}
