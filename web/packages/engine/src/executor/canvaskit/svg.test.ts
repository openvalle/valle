import { expect, test } from "bun:test";
import type { CanvasKit } from "canvaskit-wasm";
import { CanvasKitBuiltinRuntime } from "./builtin-runtime.ts";
import { isSvg, recordSvg, type SvgDocument } from "./svg.ts";

test("SVG vectors retain coverage and normalize paints into the working color domain once", async () => {
  const { default: initialize } = (await import("canvaskit-wasm")) as unknown as {
    default: (options: { locateFile: () => string }) => Promise<CanvasKit>;
  };
  const ck = await initialize({
    locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm", import.meta.dir),
  });
  const builtins = new CanvasKitBuiltinRuntime(ck);
  const doc: SvgDocument = {
    width: 2,
    height: 2,
    ops: [
      {
        kind: "draw",
        path: "M0 0L2 0L0 2Z",
        fill: { paint: { kind: "color", rgb: [255, 0, 0] }, opacity: 1, evenodd: false },
        stroke: null,
      },
    ],
  };
  const picture = recordSvg(ck, doc, (source) =>
    builtins.shader("normalizeInput", [1, 0, 100, 100], [source]),
  );
  const surface = ck.MakeSurface(256, 256)!;
  try {
    const canvas = surface.getCanvas();
    canvas.clear(ck.TRANSPARENT);
    canvas.scale(128, 128);
    canvas.drawPicture(picture);
    const pixels = canvas.readPixels(0, 0, {
      width: 256,
      height: 256,
      colorType: ck.ColorType.RGBA_8888,
      alphaType: ck.AlphaType.Premul,
      colorSpace: ck.ColorSpace.SRGB,
    }) as Uint8Array;
    const pixel = (x: number, y: number) =>
      Array.from(pixels.slice((y * 256 + x) * 4, (y * 256 + x) * 4 + 4));
    // Linear Rec.2020 red; normalization occurs inside the picture, not around its shader.
    const inside = pixel(120, 128);
    for (const [i, v] of [160, 18, 4, 255].entries())
      expect(Math.abs(inside[i] - v)).toBeLessThanOrEqual(1);
    expect(pixel(136, 128)).toEqual([0, 0, 0, 0]);
    expect(isSvg(new TextEncoder().encode("\ufeff \n<?xml version='1.0'?><svg/>"))).toBe(true);
    expect(isSvg(new Uint8Array([137, 80, 78, 71]))).toBe(false);
  } finally {
    surface.delete();
    picture.delete();
    builtins.dispose();
  }
});
