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
      await renameDirectory(output, previous);
      backedUp = true;
    }
    try { await renameDirectory(next, output); }
    catch (error) {
      if (backedUp) { await renameDirectory(previous, output); backedUp = false; }
      throw error;
    }
    await rm(scratch, { recursive: true, maxRetries: 10, retryDelay: 100 });
  } catch (error) {
    if (!backedUp) {
      try { await rm(scratch, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 }); }
      catch (cleanup) { throw new AggregateError([error, cleanup], "Build and temporary directory cleanup failed"); }
    }
    throw error;
  }
}

async function renameDirectory(from: string, to: string): Promise<void> {
  for (let attempt = 0; ; attempt++) {
    try { await rename(from, to); return; }
    catch (error) {
      if (process.platform !== "win32" || attempt >= 10
        || !["EPERM", "EBUSY", "EACCES"].includes((error as NodeJS.ErrnoException).code ?? "")) throw error;
      await new Promise(resolve => setTimeout(resolve, 100 * (attempt + 1)));
    }
  }
}

async function exists(file: string): Promise<boolean> {
  try { await stat(file); return true; }
  catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return false; throw error; }
}
