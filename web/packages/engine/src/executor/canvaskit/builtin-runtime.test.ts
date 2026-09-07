import { describe, expect, test } from "bun:test";
import type { CanvasKit } from "canvaskit-wasm";

import {
  BUILTIN_KERNELS,
  CanvasKitBuiltinRuntime,
} from "./builtin-runtime.ts";

type CanvasKitInitializer = (options: {
  locateFile(file: string): string;
}) => Promise<CanvasKit>;

const { default: CanvasKitInit } = await import("canvaskit-wasm/full") as unknown as {
  default: CanvasKitInitializer;
};
const wasmPath = Bun.resolveSync(
  "canvaskit-wasm/bin/full/canvaskit.wasm",
  import.meta.dir,
);

describe("CanvasKit built-in compositor kernels", () => {
  test("the complete shared SkSL set passes the pinned CanvasKit admission", async () => {
    const CanvasKit = await CanvasKitInit({ locateFile: () => wasmPath });
    const runtime = new CanvasKitBuiltinRuntime(CanvasKit);
    try {
      expect(() => runtime.admit(BUILTIN_KERNELS)).not.toThrow();
      expect(BUILTIN_KERNELS.length).toBeGreaterThan(0);
    } finally {
      runtime.dispose();
    }
  });
});
