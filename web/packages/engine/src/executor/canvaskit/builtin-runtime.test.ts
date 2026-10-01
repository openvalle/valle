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

test("working F16 shader stores nearest-even values through CanvasKit's truncating CPU surface", async () => {
  const ck = await CanvasKitInit({ locateFile: () => wasmPath });
  const runtime = new CanvasKitBuiltinRuntime(ck);
  runtime.admit(["workingF16"]);
  const memory = ck.Malloc(Uint8Array, 8);
  const info = { width:1, height:1, colorType:ck.ColorType.RGBA_F16, alphaType:ck.AlphaType.Premul, colorSpace:ck.ColorSpace.SRGB };
  const surface = ck.MakeRasterDirectSurface(info, memory, 8)!;
  const child = ck.Shader.MakeColor(ck.Color4f(0.5 + 1/4096, 0.5 + 3/4096, 1/3, 1), ck.ColorSpace.SRGB);
  const shader = runtime.shader("workingF16", [], [child]);
  const paint = new ck.Paint();
  try {
    paint.setShader(shader); paint.setBlendMode(ck.BlendMode.Src);
    surface.getCanvas().drawPaint(paint);
    const pixels = surface.getCanvas().readPixels(0,0,{...info,colorType:ck.ColorType.RGBA_F32})!;
    expect(Array.from(pixels).slice(0,4)).toEqual([0.5, 0.5 + 1/1024, 1365/4096, 1]);
  } finally {
    paint.delete(); shader.delete(); child.delete(); surface.delete(); ck.Free(memory); runtime.dispose();
  }
});
