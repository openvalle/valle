import { CliError } from "./args.ts";

const cwdUrl = Bun.pathToFileURL(`${process.cwd()}/`);

function directoryUrl(path: string): URL {
  const url = Bun.pathToFileURL(path);
  return new URL(url.href.endsWith("/") ? url.href : `${url.href}/`);
}

export function absolutePath(input: string): string {
  if (input.startsWith("file:")) return Bun.fileURLToPath(new URL(input));
  if (input.startsWith("/")) return Bun.fileURLToPath(Bun.pathToFileURL(input));
  return Bun.fileURLToPath(new URL(input, cwdUrl));
}

export function repositoryPath(input: string): string {
  return Bun.fileURLToPath(new URL(`../../../../${input}`, import.meta.url));
}

export function joinPath(root: string, ...parts: readonly string[]): string {
  let url = directoryUrl(root);
  for (const part of parts) url = new URL(part.replaceAll("\\", "/"), url.href.endsWith("/") ? url : directoryUrl(Bun.fileURLToPath(url)));
  return Bun.fileURLToPath(url);
}

export function directoryName(path: string): string {
  return Bun.fileURLToPath(new URL(".", Bun.pathToFileURL(path)));
}

export function relativePath(root: string, target: string): string {
  const rootParts = absolutePath(root).replaceAll("\\", "/").replace(/\/$/, "").split("/");
  const targetParts = absolutePath(target).replaceAll("\\", "/").split("/");
  while (rootParts.length && targetParts.length && rootParts[0] === targetParts[0]) {
    rootParts.shift();
    targetParts.shift();
  }
  return [...rootParts.map(() => ".."), ...targetParts].join("/") || ".";
}

export function resolvePath(root: string, input = "."): string {
  if (input.startsWith("/")) return absolutePath(input);
  return joinPath(absolutePath(root), input);
}

export function resolveRequestPath(requestDir: string, value: string): string {
  return value.startsWith("/") ? absolutePath(value) : resolvePath(requestDir, value);
}

export function resolveUnder(root: string, relative: string): string {
  if (!relative || relative.includes("\0") || relative.startsWith("/") || relative.split(/[\\/]+/).includes("..")) {
    throw new CliError(`unsafe relative path ${JSON.stringify(relative)}`, 2);
  }
  const absoluteRoot = absolutePath(root).replace(/\/$/, "");
  const resolved = resolvePath(absoluteRoot, relative);
  if (resolved !== absoluteRoot && !resolved.startsWith(`${absoluteRoot}/`)) {
    throw new CliError(`path escapes root: ${relative}`, 2);
  }
  return resolved;
}

export async function fileExists(path: string): Promise<boolean> {
  return Bun.file(absolutePath(path)).exists();
}

export function fileName(path: string): string {
  return path.replaceAll("\\", "/").split("/").at(-1) ?? path;
}

export function fileStem(path: string): string {
  const name = fileName(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}

export async function readBytes(path: string): Promise<Uint8Array> {
  const absolute = absolutePath(path);
  const file = Bun.file(absolute);
  if (!(await file.exists())) throw new CliError(`file does not exist: ${absolute}`, 2);
  return new Uint8Array(await file.arrayBuffer());
}

export async function readText(path: string): Promise<string> {
  const absolute = absolutePath(path);
  const file = Bun.file(absolute);
  if (!(await file.exists())) throw new CliError(`file does not exist: ${absolute}`, 2);
  return file.text();
}

export async function writeBytes(path: string, bytes: string | Uint8Array | ArrayBuffer | Blob): Promise<void> {
  await Bun.write(absolutePath(path), bytes);
}

export function ensureDirectory(path: string): string {
  const absolute = absolutePath(path);
  const result = Bun.spawnSync({ cmd: ["mkdir", "-p", absolute], stdout: "ignore", stderr: "pipe" });
  if (!result.success) {
    throw new CliError(
      `could not create directory ${absolute}: ${new TextDecoder().decode(result.stderr).trim()}`,
    );
  }
  return absolute;
}

export function createTempDirectory(prefix: string): string {
  const root = (process.env.TMPDIR ?? "/tmp").replace(/\/$/, "");
  const result = Bun.spawnSync({
    cmd: ["mktemp", "-d", `${root}/${prefix}.XXXXXX`],
    stdout: "pipe",
    stderr: "pipe",
  });
  if (!result.success) {
    throw new CliError(`could not create temporary directory: ${new TextDecoder().decode(result.stderr).trim()}`);
  }
  return new TextDecoder().decode(result.stdout).trim();
}

export function removeDirectory(path: string): void {
  // Temporary Chromium profile cleanup may fail while descendants exit; it does not affect test results.
  Bun.spawnSync({ cmd: ["rm", "-rf", "--", absolutePath(path)], stderr: "pipe" });
}
