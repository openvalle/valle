import { readdir } from "node:fs/promises";
import path from "node:path";

/** Notices travel with both the local runtime and the separately distributed SDK. */
export async function thirdPartyNotices(repo: string): Promise<string> {
  const notices = [await Bun.file(path.join(repo, "LICENSE")).text()];
  for (const relative of ["assets/fonts", "crates/valle-motion/licenses", "crates/valle-draw/licenses"]) {
    for (const file of (await textFiles(path.join(repo, relative))).sort()) {
      notices.push(`${path.relative(repo, file)}\n\n${await Bun.file(file).text()}`);
    }
  }
  return notices.join("\n\n");
}

async function textFiles(dir: string): Promise<string[]> {
  const files: string[] = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) files.push(...await textFiles(file));
    else if (file.endsWith(".txt")) files.push(file);
  }
  return files;
}
