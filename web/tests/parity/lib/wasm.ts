import { CliError } from "./args.ts";
import { readBytes } from "./files.ts";

export type WasmFunction = (...args: number[]) => number;

export async function instantiateWasm(path: string): Promise<WebAssembly.Instance> {
  const bytes = await readBytes(path);
  const result = await WebAssembly.instantiate(bytes, {});
  return result.instance;
}

export function requireWasmFunction(
  instance: WebAssembly.Instance,
  name: string,
): WasmFunction {
  const value = instance.exports[name];
  if (typeof value !== "function") throw new CliError(`WASM export ${name} is missing`);
  return value as WasmFunction;
}
