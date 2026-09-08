import { describe, expect, test } from "bun:test";
import { resolve } from "node:path";
import type { CanvasKit } from "canvaskit-wasm";

import {
  CANVASKIT_GLYPH_COVERAGE_PROFILE,
  CanvasKitExecutor,
  makePaint,
  applyClip,
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


test("two-circle gradient keeps its offset focal point", async () => {
  const ck = await CanvasKitInit({ locateFile: () => wasmPath });
  const surface = ck.MakeSurface(24, 24)!;
  const paint = makePaint(ck, { kind: "twoCircleGradient", value: {
    start: [4, 12], startRadius: 0, end: [12, 12], endRadius: 12, spread: "pad",
    stops: [
      { offset: 0, color: { red: 1, green: 0, blue: 0, alpha: 1 } },
      { offset: 1, color: { red: 0, green: 0, blue: 1, alpha: 1 } },
    ],
  }});
  try {
    surface.getCanvas().drawRect(ck.XYWHRect(0, 0, 24, 24), paint);
    const image = surface.makeImageSnapshot();
    try {
      const pixels = image.readPixels(0, 0, {
        width: 24, height: 24, colorType: ck.ColorType.RGBA_8888,
        alphaType: ck.AlphaType.Unpremul, colorSpace: ck.ColorSpace.SRGB,
      })!;
      const focal = (12 * 24 + 4) * 4, edge = (12 * 24 + 23) * 4;
      expect(pixels[focal]!).toBeGreaterThan(pixels[focal + 2]!);
      expect(pixels[edge + 2]!).toBeGreaterThan(pixels[edge]!);
      expect(pixels[focal + 3]).toBe(255);
    } finally { image.delete(); }
  } finally { paint.delete(); surface.delete(); }
});


test("transformed clip preserves device-space image coordinates", async () => {
  const ck = await CanvasKitInit({ locateFile: () => wasmPath });
  const surface = ck.MakeSurface(16, 16)!;
  const paint = new ck.Paint();
  paint.setColor(ck.RED);
  try {
    const draw = { paths: [{ verbs: ["moveTo", "lineTo", "lineTo", "close"],
      points: [[0, 0], [8, 0], [0, 8]] }] } as Parameters<typeof applyClip>[2];
    applyClip(ck, surface.getCanvas(), draw,
      { kind: "path", value: { path: 0, fillRule: "nonZero" } },
      [1, 0, 4, 0, -1, 12, 0, 0, 1]);
    surface.getCanvas().drawRect(ck.XYWHRect(4, 4, 8, 8), paint);
    const image = surface.makeImageSnapshot();
    try {
      const pixels = image.readPixels(0, 0, { width: 16, height: 16,
        colorType: ck.ColorType.RGBA_8888, alphaType: ck.AlphaType.Unpremul,
        colorSpace: ck.ColorSpace.SRGB })!;
      expect(pixels[(10 * 16 + 5) * 4 + 3]).toBe(255);
      expect(pixels[(2 * 16 + 5) * 4 + 3]).toBe(0);
    } finally { image.delete(); }
  } finally { paint.delete(); surface.delete(); }
});
