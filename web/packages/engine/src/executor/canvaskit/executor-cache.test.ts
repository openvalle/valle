import { describe, expect, test } from "bun:test";
import { resolve } from "node:path";
import type { CanvasKit } from "canvaskit-wasm";

import {
  CANVASKIT_GLYPH_COVERAGE_PROFILE,
  CanvasKitExecutor,
} from "./executor.ts";

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

describe("long-lived CanvasKit executor caches", () => {
  test("glyph coverage profile identifies the embedded renderer and frozen knobs", () => {
    expect(CANVASKIT_GLYPH_COVERAGE_PROFILE).toEqual({
      rasterizer: "freetype",
      library: "renderer-embedded",
      hinting: "none",
      edging: "antialias",
      subpixelPositioning: false,
    });
  });

  test("font and runtime shader admission objects are reused across frames", async () => {
    const CanvasKit = await CanvasKitInit({ locateFile: () => wasmPath });
    const executor = new CanvasKitExecutor(CanvasKit);
    const font = new Uint8Array(await Bun.file(resolve(
      import.meta.dir,
      "../../../../../../crates/valle-motion/assets/fonts/noto/NotoSansCJKsc-Regular.otf",
    )).arrayBuffer());
    const shader = new TextEncoder().encode("half4 main(float2 xy) { return half4(1); }");
    try {
      expect(executor.typeface("font:0", 0, font)).toBe(executor.typeface("font:0", 0, font));
      expect(() => executor.typeface("font:1", 1, font)).toThrow(
        "font 'font:1' requests unsupported collection face 1",
      );
      expect(executor.runtimeShader("shader:1", shader)).toBe(executor.runtimeShader("shader:1", shader));
      expect(executor.cacheSnapshot()).toEqual({
        programHits: 0,
        programMisses: 0,
        fontHits: 1,
        fontMisses: 1,
        shaderHits: 1,
        shaderMisses: 1,
      });
    } finally {
      executor.dispose();
    }
  });
});
