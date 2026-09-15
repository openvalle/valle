import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { initSync, ProductEngine } from "../../../generated/web/valle_engine.js";
import { CanvasKitBuiltinRuntime } from "./builtin-runtime.ts";
import { drawSrgbPreviewCpu } from "./output-transform.ts";
import type { CanvasKit } from "canvaskit-wasm";

const { default: init } = await import("canvaskit-wasm/full") as unknown as {
  default: (options: { locateFile(): string }) => Promise<CanvasKit>;
};
const ck = await init({ locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/full/canvaskit.wasm", import.meta.dir) });
initSync({ module: await readFile(new URL("../../../generated/web/valle_engine_bg.wasm", import.meta.url)) });

test("CPU SDR output matches shared SkSL across gamut, near-black and coverage boundaries", () => {
  const engine = new ProductEngine();
  const builtins = new CanvasKitBuiltinRuntime(ck);
  const colors = [[0,0,0], [1,1,1], [1,0,0], [0,1,0], [0,0,1], [-.2,.5,1.4], [2,-1,.1]];
  for (let i = 0; i <= 512; i++) colors.push([i / 512, i / 2048, 1 - i / 512]);
  for (const v of [0.000001, 0.001, 0.0031307, 0.0031308, 0.0031309, 0.018]) colors.push([v,v,v]);
  const pixels = new Float32Array(colors.flatMap(rgb => [0, 1e-8, 1e-6, 1/255, .01, .25, .5, 1].flatMap(a => [...rgb.map(v => v*a), a])));
  const width = pixels.length / 4, height = 1;
  const info = { width, height, colorType: ck.ColorType.RGBA_F16, alphaType: ck.AlphaType.Premul, colorSpace: ck.ColorSpace.SRGB };
  const allocations = [0,1,2].map(() => ck.Malloc(Uint8Array, width * 8));
  const surfaces = allocations.map(m => ck.MakeRasterDirectSurface(info, m, width * 8)!);
  const [source, reference, actual] = surfaces;
  let image: ReturnType<typeof source.makeImageSnapshot> | null = null;
  try {
    expect(source!.getCanvas().writePixels(new Uint8Array(pixels.buffer), width, height, 0, 0, ck.AlphaType.Premul, ck.ColorType.RGBA_F32, ck.ColorSpace.SRGB)).toBe(true);
    image = source!.makeImageSnapshot();
    for (const opaque of [false, true]) {
      const child = image.makeShaderOptions(ck.TileMode.Clamp, ck.TileMode.Clamp, ck.FilterMode.Linear, ck.MipmapMode.None);
      const shader = builtins.shader("outputTransform", [0,1,0,1,opaque ? 0 : 1,100,100], [child]);
      const paint = new ck.Paint(); paint.setShader(shader); paint.setBlendMode(ck.BlendMode.Src);
      try {
        reference!.getCanvas().drawPaint(paint);
        drawSrgbPreviewCpu(ck, actual!.getCanvas(), image, engine, opaque);
        for (const alphaType of [ck.AlphaType.Premul, ck.AlphaType.Unpremul]) {
          const read = (surface: typeof reference) => {
            const target = ck.MakeSurface(width, height)!;
            const input = surface!.makeImageSnapshot();
            const p = new ck.Paint(); p.setBlendMode(ck.BlendMode.Src);
            target.getCanvas().drawImage(input, 0, 0, p);
            const result = target.makeImageSnapshot();
            try { return Uint8Array.from(result.readPixels(0,0,{width,height,colorType:ck.ColorType.RGBA_8888,alphaType,colorSpace:ck.ColorSpace.SRGB})!); }
            finally { result.delete(); input.delete(); p.delete(); target.delete(); }
          };
          const a = read(actual), b = read(reference);
          let max = 0;
          for (let i=0;i<a.length;i++) { max=Math.max(max,Math.abs(a[i]!-b[i]!)); }
          // Existing Native/CanvasKit RGBA parity budget; never allow alpha to drift.
          expect(max).toBeLessThanOrEqual(2);
          expect(a.filter((_, i) => i % 4 === 3)).toEqual(b.filter((_, i) => i % 4 === 3));
        }
      } finally { paint.delete(); shader.delete(); child.delete(); }
    }
  } finally { image?.delete(); for (const s of surfaces) s.delete(); for (const m of allocations) ck.Free(m); builtins.dispose(); engine.free(); }
});

test("CPU output rejects malformed and nonfinite pixels", () => {
  const engine = new ProductEngine();
  try {
    expect(() => engine.transform_srgb_preview_pixels(new Float32Array(3), false)).toThrow();
    expect(() => engine.transform_srgb_preview_pixels(new Float32Array([NaN,0,0,1]), false)).toThrow();
    const transparent = new Float32Array([1e-6,0,0,0]);
    engine.transform_srgb_preview_pixels(transparent, false);
    expect(transparent).toEqual(new Float32Array(4));
  } finally { engine.free(); }
});
