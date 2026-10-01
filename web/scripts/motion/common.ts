import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

export const ROOT = resolve(import.meta.dir, "../../..");

export function run(argv: string[], options: { cwd?: string; input?: Uint8Array } = {}): string {
  const result = spawnSync(argv[0]!, argv.slice(1), {
    cwd: options.cwd ?? ROOT,
    input: options.input,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error || result.status !== 0) {
    throw new Error(`${argv.join(" ")} failed (${result.status}):\n${result.stderr?.slice(-3000) ?? ""}\n${result.stdout?.slice(-2000) ?? ""}`, {
      cause: result.error,
    });
  }
  return result.stdout;
}

export function runJson<T>(argv: string[], options?: { cwd?: string }): T {
  return JSON.parse(run(argv, options)) as T;
}

export function option(args: string[], name: string, fallback?: string): string {
  const index = args.indexOf(name);
  if (index < 0) {
    if (fallback === undefined) throw new Error(`missing ${name}`);
    return fallback;
  }
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`missing value for ${name}`);
  return value;
}

export function nonnegativeInteger(value: string, name: string, minimum = 0): number {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < minimum) throw new Error(`${name} must be an integer >= ${minimum}`);
  return parsed;
}

export function median(values: number[]): number {
  if (!values.length) throw new Error("median requires samples");
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2;
}

export function sha256(path: string): string {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}
