import type { CommandDefinition } from "../lib/args.ts";
import { requiredOption } from "../lib/args.ts";
import { readText } from "../lib/files.ts";
import { instantiateWasm, requireWasmFunction } from "../lib/wasm.ts";

export const mathCommand: CommandDefinition = {
  name: "math",
  summary: "Compare the deterministic-math WASM corpus hash with its checked-in golden.",
  options: [
    { name: "wasm", value: "module.wasm", description: "deterministic_math_wasm module", required: true },
    { name: "golden", value: "golden.sha256", description: "expected SHA-256 text file", required: true },
  ],
  async run(options) {
    const instance = await instantiateWasm(requiredOption(options, "wasm"));
    const byteAt = requireWasmFunction(instance, "deterministic_math_hash_byte");
    const actual = Array.from({ length: 32 }, (_, index) =>
      byteAt(index).toString(16).padStart(2, "0"),
    ).join("");
    const expected = (await readText(requiredOption(options, "golden"))).trim();
    if (actual !== expected) {
      throw new Error(`deterministic math corpus mismatch: wasm=${actual} golden=${expected}`);
    }
    console.log(`deterministic math wasm corpus: ${actual} ✓`);
  },
};
