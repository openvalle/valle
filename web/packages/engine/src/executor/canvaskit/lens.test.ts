import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import type { CanvasKit } from "canvaskit-wasm";
import { initSync, ProductEngine } from "../../../generated/web/valle_engine.js";
import { CanvasKitBuiltinRuntime } from "./builtin-runtime.ts";
import { drawLensDistortion } from "./lens.ts";

initSync({
  module: await readFile(new URL("../../../generated/web/valle_engine_bg.wasm", import.meta.url)),
});
const { default: CanvasKitInit } = (await import("canvaskit-wasm")) as unknown as {
  default: (options: { locateFile(file: string): string }) => Promise<CanvasKit>;
};
const ck = await CanvasKitInit({
  locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm", import.meta.dir),
});

test("GPU lens keeps CPU subpixel sampling, logical decal bounds and crop coordinates", () => {
  const engine = new ProductEngine(),
    runtime = new CanvasKitBuiltinRuntime(ck);
  runtime.admit(["lensDistortion"]);
  const iw = 113,
    ih = 79,
    w = 91,
    h = 67;
  const bytes = new Uint8Array(iw * ih * 8),
    view = new DataView(bytes.buffer);
  for (let y = 0; y < ih; y++)
    for (let x = 0; x < iw; x++) {
      const values =
        (x + y) % 7 < 3
          ? [0x3c00, 0x3800, 0x3400, 0x3c00]
          : x % 11 < 4
            ? [0x4000, 0x3c00, 0x3800, 0x3c00]
            : [0, 0, 0, 0];
      values.forEach((v, c) => view.setUint16((y * iw + x) * 8 + c * 2, v, true));
    }
  const info = {
    width: iw,
    height: ih,
    colorType: ck.ColorType.RGBA_F16,
    alphaType: ck.AlphaType.Premul,
    colorSpace: ck.ColorSpace.SRGB,
  };
  const input = ck.MakeImage(info, bytes, iw * 8)!;
  const memory = ck.Malloc(Uint8Array, w * h * 8);
  const surface = ck.MakeRasterDirectSurface({ ...info, width: w, height: h }, memory, w * 8)!;
  try {
    const cases = [
      { origin: [0, 0], offset: [-8, 7], frame: [0, 0, 140, 100] },
      {
        origin: [-43, 31],
        offset: [15, -6],
        frame: [-32.25, 15.5, 109.75, 96.25],
      },
      { origin: [1870, 1019], offset: [-15, -12], frame: [0, 0, 1920, 1080] },
      {
        origin: [9999923, -9999803],
        offset: [-8, 7],
        frame: [9999910, -9999840, 115.5, 132.25],
      },
    ];
    for (const { origin, offset, frame } of cases)
      for (const [k1, k2] of [
        [0, 0],
        [0.35, -0.1],
        [-0.45, 0.2],
      ]) {
        const output = { x: origin[0]!, y: origin[1]!, width: w, height: h };
        const roi = {
          x: output.x + offset[0]!,
          y: output.y + offset[1]!,
          width: iw,
          height: ih,
        };
        const canvas = surface.getCanvas();
        canvas.clear(ck.TRANSPARENT);
        canvas.save();
        canvas.translate(-output.x, -output.y);
        try {
          drawLensDistortion(ck, runtime, canvas, input, roi, output, frame, k1!, k2!);
        } finally {
          canvas.restore();
        }
        const actual = canvas.readPixels(0, 0, {
          ...info,
          width: w,
          height: h,
          colorType: ck.ColorType.RGBA_F32,
        }) as Float32Array;
        const expected = engine.apply_lens_distortion_f16(
          bytes,
          iw,
          ih,
          w,
          h,
          offset[0]!,
          offset[1]!,
          output.x,
          output.y,
          k1!,
          k2!,
          frame[0]!,
          frame[1]!,
          frame[2]!,
          frame[3]!,
        );
        const image = ck.MakeImage({ ...info, width: w, height: h }, expected, w * 8)!;
        try {
          const pixels = image.readPixels(0, 0, {
            ...info,
            width: w,
            height: h,
            colorType: ck.ColorType.RGBA_F32,
          }) as Float32Array;
          let max = 0;
          for (let i = 0; i < w * h * 4; i++)
            max = Math.max(max, Math.abs(actual[i]! - pixels[i]!));
          expect(max, `origin=${origin} offset=${offset} k=${k1},${k2}`).toBeLessThanOrEqual(0.001);
        } finally {
          image.delete();
        }
      }
    expect(() =>
      drawLensDistortion(
        ck,
        runtime,
        surface.getCanvas(),
        input,
        { x: 0, y: 0, width: iw, height: ih },
        { x: 0, y: 0, width: w, height: h },
        [0, 0, 0, h],
        0.1,
        0,
      ),
    ).toThrow();
  } finally {
    input.delete();
    surface.delete();
    ck.Free(memory);
    runtime.dispose();
    engine.free();
  }
}, 30_000);
