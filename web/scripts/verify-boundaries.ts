import { readdir } from "node:fs/promises";
import path from "node:path";

const root = path.resolve(import.meta.dir, "..");
const failures: string[] = [];

for (const file of await sourceFiles([path.join(root, "packages"), path.join(root, "apps")])) {
  const source = await Bun.file(file).text();
  const relative = path.relative(root, file);
  if (/apps\/[^/]+\/src\//.test(relative)) {
    if (/from\s+["'][^"']*packages\//.test(source) || /@valle\/[^"']+\/src\//.test(source)) {
      failures.push(`${relative}: app bypasses a package export`);
    }
  }
}

// Reject Node APIs that have direct Bun replacements and browser-incompatible Buffer usage. Retain
// supported Node built-ins for path and filesystem operations without equivalent Bun APIs.
const nodeLeftovers = [
  ["node:assert", /from\s+["']node:assert/, "bun:test expect with custom messages"],
  ["require()", /\brequire\(\s*["']/, "await import()"],
  ["Buffer", /\bBuffer\.(?:from|alloc|concat)\(/, "Uint8Array / Uint8Array.fromBase64()"],
] as const;
for (const file of await sourceFiles([path.join(root, "packages"), path.join(root, "apps"), path.join(root, "scripts")])) {
  // Skip wasm-bindgen output, which may contain generated CommonJS bindings.
  if (file.includes(`${path.sep}generated${path.sep}`)) continue;
  const source = await Bun.file(file).text();
  for (const [name, pattern, replacement] of nodeLeftovers) {
    if (pattern.test(source)) {
      failures.push(`${path.relative(root, file)}: uses ${name}; replace with ${replacement}`);
    }
  }
}

const appEntries = await readdir(path.join(root, "apps"), { withFileTypes: true });
const studioApps = appEntries.filter((entry) => entry.isDirectory() && entry.name.includes("studio"));
if (studioApps.length !== 1 || studioApps[0]?.name !== "studio") {
  failures.push(`apps must contain exactly one Studio, found: ${studioApps.map((entry) => entry.name).join(", ")}`);
}

if (failures.length > 0) {
  throw new Error(`web boundary verification failed:\n- ${failures.join("\n- ")}`);
}
console.log("web boundaries verified");

async function sourceFiles(roots: string[]): Promise<string[]> {
  const files: string[] = [];
  for (const dir of roots) {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const absolute = path.join(dir, entry.name);
      if (entry.isDirectory()) files.push(...await sourceFiles([absolute]));
      else if (/\.(?:ts|js|mjs|html)$/.test(entry.name)) files.push(absolute);
    }
  }
  return files;
}
