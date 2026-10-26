import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { initSync, ProductEngine } from "../../../generated/web/valle_engine.js";
import { CanvasKitBuiltinRuntime } from "./builtin-runtime.ts";
import {
  drawSrgbPreviewCpu,
  srgbPreviewScratchBytes,
  outputDitherPixels,
} from "./output-transform.ts";
import type { CanvasKit } from "canvaskit-wasm";

const { default: init } = (await import("canvaskit-wasm")) as unknown as {
  default: (options: { locateFile(): string }) => Promise<CanvasKit>;
};
const ck = await init({
  locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm", import.meta.dir),
});

test("delivery noise matches Native's bounded seeded TPDF table", () => {
  expect(new Bun.CryptoHasher("sha256").update(outputDitherPixels(0)).digest("hex")).toBe(
    "3155af14f33803983c8af3cbda65e595791a1918e721b83675eac07686ea3f87",
  );
});
initSync({
  module: await readFile(new URL("../../../generated/web/valle_engine_bg.wasm", import.meta.url)),
});

test("CPU SDR output matches shared SkSL across gamut, near-black and coverage boundaries", async () => {
  const engine = new ProductEngine();
  const builtins = new CanvasKitBuiltinRuntime(ck);
  const colors = [
    [0, 0, 0],
    [1, 1, 1],
    [1, 0, 0],
    [0, 1, 0],
    [0, 0, 1],
    [-0.2, 0.5, 1.4],
    [2, -1, 0.1],
  ];
  for (let i = 0; i <= 512; i++) colors.push([i / 512, i / 2048, 1 - i / 512]);
  for (const v of [0.000001, 0.001, 0.0031307, 0.0031308, 0.0031309, 0.018]) colors.push([v, v, v]);
  const pixels = new Float32Array(
    colors.flatMap((rgb) =>
      [0, 1e-8, 1e-6, 1 / 255, 0.01, 0.25, 0.5, 1].flatMap((a) => [...rgb.map((v) => v * a), a]),
    ),
  );
  const width = pixels.length / 4,
    height = 1;
  const info = {
    width,
    height,
    colorType: ck.ColorType.RGBA_F16,
    alphaType: ck.AlphaType.Premul,
    colorSpace: ck.ColorSpace.SRGB,
  };
  const allocations = [0, 1, 2].map(() => ck.Malloc(Uint8Array, width * 8));
  const surfaces = allocations.map((m) => ck.MakeRasterDirectSurface(info, m, width * 8)!);
  const [source, reference, actual] = surfaces;
  let image: ReturnType<typeof source.makeImageSnapshot> | null = null;
  try {
    expect(
      source!
        .getCanvas()
        .writePixels(
          new Uint8Array(pixels.buffer),
          width,
          height,
          0,
          0,
          ck.AlphaType.Premul,
          ck.ColorType.RGBA_F32,
          ck.ColorSpace.SRGB,
        ),
    ).toBe(true);
    image = source!.makeImageSnapshot();
    for (const opaque of [false, true]) {
      const child = image.makeShaderOptions(
        ck.TileMode.Clamp,
        ck.TileMode.Clamp,
        ck.FilterMode.Linear,
        ck.MipmapMode.None,
      );
      const noise = builtins.outputNoise(null);
      const shader = builtins.shader(
        "outputTransform",
        [0, 1, 0, 1, opaque ? 0 : 1, 100, 100, 0],
        [child, noise],
      );
      noise.delete();
      const paint = new ck.Paint();
      paint.setShader(shader);
      paint.setBlendMode(ck.BlendMode.Src);
      try {
        reference!.getCanvas().drawPaint(paint);
        await drawSrgbPreviewCpu(ck, actual!.getCanvas(), image, engine, opaque);
        for (const alphaType of [ck.AlphaType.Premul, ck.AlphaType.Unpremul]) {
          const read = (surface: typeof reference) => {
            const target = ck.MakeSurface(width, height)!;
            const input = surface!.makeImageSnapshot();
            const p = new ck.Paint();
            p.setBlendMode(ck.BlendMode.Src);
            target.getCanvas().drawImage(input, 0, 0, p);
            const result = target.makeImageSnapshot();
            try {
              return Uint8Array.from(
                result.readPixels(0, 0, {
                  width,
                  height,
                  colorType: ck.ColorType.RGBA_8888,
                  alphaType,
                  colorSpace: ck.ColorSpace.SRGB,
                })!,
              );
            } finally {
              result.delete();
              input.delete();
              p.delete();
              target.delete();
            }
          };
          const a = read(actual),
            b = read(reference);
          let max = 0;
          for (let i = 0; i < a.length; i++) {
            max = Math.max(max, Math.abs(a[i]! - b[i]!));
          }
          // Existing Native/CanvasKit RGBA parity budget; never allow alpha to drift.
          expect(max).toBeLessThanOrEqual(2);
          expect(a.filter((_, i) => i % 4 === 3)).toEqual(b.filter((_, i) => i % 4 === 3));
        }
      } finally {
        paint.delete();
        shader.delete();
        child.delete();
      }
    }
  } finally {
    image?.delete();
    for (const s of surfaces) s.delete();
    for (const m of allocations) ck.Free(m);
    builtins.dispose();
    engine.free();
  }
});

test("CPU output rejects malformed and nonfinite pixels", async () => {
  const engine = new ProductEngine();
  try {
    expect(() => engine.transform_srgb_preview_pixels(new Float32Array(3), false)).toThrow();
    expect(() =>
      engine.transform_srgb_preview_pixels(new Float32Array([NaN, 0, 0, 1]), false),
    ).toThrow();
    expect(() => engine.transform_srgb_preview_f16(new Uint8Array(7), false)).toThrow();
    expect(() =>
      engine.transform_srgb_preview_f16(new Uint8Array([0, 0x7c, 0, 0, 0, 0, 0, 0x3c]), false),
    ).toThrow();
    const transparent = new Float32Array([1e-6, 0, 0, 0]);
    engine.transform_srgb_preview_pixels(transparent, false);
    expect(transparent).toEqual(new Float32Array(4));
  } finally {
    engine.free();
  }
});

test("F16 output preserves the float read/convert/write result across stripes", async () => {
  const engine = new ProductEngine();
  const width = 17,
    height = 129;
  const info = {
    width,
    height,
    colorType: ck.ColorType.RGBA_F16,
    alphaType: ck.AlphaType.Premul,
    colorSpace: ck.ColorSpace.SRGB,
  };
  const allocations = [0, 1, 2].map(() => ck.Malloc(Uint8Array, width * height * 8));
  const surfaces = allocations.map((memory) =>
    ck.MakeRasterDirectSurface(info, memory, width * 8)!,
  );
  let image: ReturnType<(typeof surfaces)[0]["makeImageSnapshot"]> | null = null;
  try {
    const colors = [
      [0, 0, 0],
      [1, 1, 1],
      [-0.2, 0.5, 1.4],
      [2, -1, 0.1],
      [0.0031307, 0.0031308, 0.0031309],
      [1e-6, 1e-4, 0.001],
    ];
    const alpha = [0, 1e-8, 1e-6, 1 / 255, 0.25, 0.5, 1];
    const pixels = new Float32Array(
      Array.from({ length: width * height }, (_, i) => {
        const a = alpha[Math.floor(i / 11) % alpha.length]!;
        return [...colors[Math.floor(i / 3) % colors.length]!.map((value) => value * a), a];
      }).flat(),
    );
    expect(
      surfaces[0]!
        .getCanvas()
        .writePixels(
          new Uint8Array(pixels.buffer),
          width,
          height,
          0,
          0,
          ck.AlphaType.Premul,
          ck.ColorType.RGBA_F32,
          ck.ColorSpace.SRGB,
        ),
    ).toBe(true);
    image = surfaces[0]!.makeImageSnapshot();
    const floatKernel = {
      transform_srgb_preview_pixels: engine.transform_srgb_preview_pixels.bind(engine),
    };
    const read = (surface: (typeof surfaces)[number]) => {
      const snapshot = surface.makeImageSnapshot();
      try {
        return Uint8Array.from(snapshot.readPixels(0, 0, info, undefined, width * 8)!);
      } finally {
        snapshot.delete();
      }
    };
    for (const opaque of [false, true]) {
      expect(
        await drawSrgbPreviewCpu(ck, surfaces[1]!.getCanvas(), image, floatKernel, opaque),
      ).toBe(true);
      expect(await drawSrgbPreviewCpu(ck, surfaces[2]!.getCanvas(), image, engine, opaque)).toBe(
        true,
      );
      expect(read(surfaces[2]!)).toEqual(read(surfaces[1]!));
    }
    expect(srgbPreviewScratchBytes(3840, 2160, true)).toBe(1_966_080n);
  } finally {
    image?.delete();
    for (const surface of surfaces) surface.delete();
    for (const memory of allocations) ck.Free(memory);
    engine.free();
  }
});

for (const f16 of [false, true])
  test(`coverage probe rejects partial pixels without drawing (${f16 ? "F16" : "F32"})`, async () => {
    const width = 3,
      height = 129;
    const info = {
      width,
      height,
      colorType: ck.ColorType.RGBA_F16,
      alphaType: ck.AlphaType.Premul,
      colorSpace: ck.ColorSpace.SRGB,
    };
    const memory = ck.Malloc(Uint8Array, width * height * 8);
    const source = ck.MakeRasterDirectSurface(info, memory, 3 * 8)!;
    const target = ck.MakeSurface(width, height)!;
    let image: ReturnType<typeof source.makeImageSnapshot> | null = null;
    let calls = 0;
    let transformedPixels = 0;
    const engine = new ProductEngine();
    const kernel = {
      transform_srgb_preview_pixels(pixels: Float32Array, opaque: boolean) {
        calls++;
        transformedPixels += pixels.length / 4;
        expect(pixels.length).toBeLessThanOrEqual(width * 64 * 4);
        engine.transform_srgb_preview_pixels(pixels, opaque);
      },
      ...(f16
        ? {
            transform_srgb_preview_f16(pixels: Uint8Array, opaque: boolean) {
              calls++;
              transformedPixels += pixels.length / 8;
              expect(pixels.length).toBeLessThanOrEqual(width * 64 * 8);
              engine.transform_srgb_preview_f16(pixels, opaque);
            },
          }
        : {}),
    };
    const readTarget = () => {
      const snapshot = target.makeImageSnapshot();
      try {
        return Uint8Array.from(
          snapshot.readPixels(0, 0, {
            width,
            height,
            colorType: ck.ColorType.RGBA_8888,
            alphaType: ck.AlphaType.Unpremul,
            colorSpace: ck.ColorSpace.SRGB,
          })!,
        );
      } finally {
        snapshot.delete();
      }
    };
    try {
      target.getCanvas().clear(ck.Color4f(0.1, 0.2, 0.3, 0.5));
      const before = readTarget();
      for (const alpha of [0, 1e-6, 0.5, 0.99951171875]) {
        const pixels = new Float32Array(
          Array.from({ length: width * height }, () => [0.1, 0.2, 0.3, 1]).flat(),
        );
        pixels.set([0.1 * alpha, 0.2 * alpha, 0.3 * alpha, alpha], pixels.length - 4);
        source
          .getCanvas()
          .writePixels(
            new Uint8Array(pixels.buffer),
            width,
            height,
            0,
            0,
            ck.AlphaType.Premul,
            ck.ColorType.RGBA_F32,
            ck.ColorSpace.SRGB,
          );
        image = source.makeImageSnapshot();
        expect(await drawSrgbPreviewCpu(ck, target.getCanvas(), image, kernel, false, true)).toBe(
          false,
        );
        expect(readTarget()).toEqual(before);
        image.delete();
        image = null;
      }
      expect(calls).toBe(0);
      source.getCanvas().clear(ck.Color4f(0.1, 0.2, 0.3, 1));
      image = source.makeImageSnapshot();
      expect(await drawSrgbPreviewCpu(ck, target.getCanvas(), image, kernel, false, true)).toBe(
        true,
      );
      expect(calls).toBeGreaterThan(1);
      expect(transformedPixels).toBe(width * height);
      expect(readTarget().filter((_, i) => i % 4 === 3)).toEqual(
        new Uint8Array(width * height).fill(255),
      );
    } finally {
      image?.delete();
      source.delete();
      target.delete();
      ck.Free(memory);
      engine.free();
    }
  });
