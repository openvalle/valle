import { describe, expect, test } from "bun:test";
import type { CanvasKit } from "canvaskit-wasm";

import { CanvasKitBuiltinRuntime } from "./builtin-runtime.ts";
import { applyPreparedEffect } from "./executor.ts";

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

describe("CanvasKit prepared effect write domains", () => {
  test("a transformed layer region samples the immutable full input and only replaces its clip", async () => {
    const CanvasKit = await CanvasKitInit({ locateFile: () => wasmPath });
    const builtins = new CanvasKitBuiltinRuntime(CanvasKit);
    const input = CanvasKit.MakeImage(
      {
        width: 3,
        height: 1,
        colorType: CanvasKit.ColorType.RGBA_8888,
        alphaType: CanvasKit.AlphaType.Unpremul,
        colorSpace: CanvasKit.ColorSpace.SRGB,
      },
      Uint8Array.from([
        255, 0, 0, 255,
        0, 0, 0, 0,
        0, 0, 0, 0,
      ]),
      12,
    );
    const surface = CanvasKit.MakeSurface(3, 1);
    if (!input || !surface) throw new Error("CanvasKit test surface allocation failed");

    const dynamic = new Map<number, Record<string, unknown>>([
      [1, { value: { kind: "scalar", value: 1 } }],
      [2, { value: { kind: "deviceTransform", value: { matrix: [3, 0, 0, 0, 1, 0, 0, 0, 1] } } }],
      [3, { value: { kind: "bounds", value: { x: 0, y: 0, width: 3, height: 1 } } }],
    ]);
    const effect = {
      semanticPath: "test.layer.blur",
      space: { kind: "layer", transform: 2, bounds: 3 },
      kernel: {
        kind: "gaussianBlur",
        sigmaDevicePx: 1,
        region: { x: 1 / 3, y: 0, width: 1 / 3, height: 1 },
      },
    };

    try {
      const canvas = surface.getCanvas();
      canvas.clear(CanvasKit.TRANSPARENT);
      applyPreparedEffect(CanvasKit, builtins, canvas, input, effect, dynamic, { width: 3, height: 1 });
      surface.flush();
      const pixels = canvas.readPixels(0, 0, {
        width: 3,
        height: 1,
        colorType: CanvasKit.ColorType.RGBA_8888,
        alphaType: CanvasKit.AlphaType.Unpremul,
        colorSpace: CanvasKit.ColorSpace.SRGB,
      });
      expect(pixels).not.toBeNull();
      const rgba = [...pixels!];
      expect(rgba.slice(0, 4)).toEqual([255, 0, 0, 255]);
      expect(rgba[4]).toBeGreaterThan(0);
      expect(rgba[7]).toBeGreaterThan(0);
      expect(rgba[7]).toBeLessThan(255);
      expect(rgba.slice(8, 12)).toEqual([0, 0, 0, 0]);
    } finally {
      input.delete();
      surface.delete();
      builtins.dispose();
    }
  });
});
