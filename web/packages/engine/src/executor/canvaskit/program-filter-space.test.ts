import { describe, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import type { Canvas, CanvasKit } from "canvaskit-wasm";

import noiseDisplacementSource from "../../../../../../crates/valle-draw/assets/shaders/noisedisplacement.sksl" with { type: "text" };
import { initSync, ProductEngine } from "../../../generated/web/valle_engine.js";
import { CanvasKitBuiltinRuntime } from "./builtin-runtime.ts";
import {
  backdropFilterMatrix,
  backdropFiltersInDeviceSpace,
  backdropSimilarity,
  drawNoiseDisplacement,
  programFilterInDeviceSpace,
} from "./executor.ts";

initSync({
  module: await readFile(new URL("../../../generated/web/valle_engine_bg.wasm", import.meta.url)),
});

type CanvasKitInitializer = (options: { locateFile(file: string): string }) => Promise<CanvasKit>;

const { default: CanvasKitInit } = (await import("canvaskit-wasm")) as unknown as {
  default: CanvasKitInitializer;
};
const wasmPath = Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm", import.meta.dir);

describe("scheduled DrawProgram filter space", () => {
  test("maps caption similarity transforms into device pixels", () => {
    expect(
      programFilterInDeviceSpace(
        {
          kind: "dropShadow",
          offset: [2, 3],
          sigmaX: 5,
          sigmaY: 5,
          color: { red: 0, green: 0, blue: 0, alpha: 1 },
        },
        [0, -2, 8, 2, 0, 9, 0, 0, 1],
      ),
    ).toEqual({
      kind: "dropShadow",
      offset: [-6, 4],
      sigmaX: 10,
      sigmaY: 10,
      color: { red: 0, green: 0, blue: 0, alpha: 1 },
    });

    expect(
      programFilterInDeviceSpace(
        { kind: "blur", sigmaX: 3, sigmaY: 3 },
        [-2, 0, 0, 0, 2, 0, 0, 0, 1],
      ),
    ).toEqual({ kind: "blur", sigmaX: 6, sigmaY: 6 });
  });

  test("accepts an exactly representable axis-aligned nonuniform mapping", () => {
    expect(
      programFilterInDeviceSpace(
        { kind: "blur", sigmaX: 5, sigmaY: 5 },
        [2, 0, 0, 0, 3, 0, 0, 0, 1],
      ),
    ).toEqual({ kind: "blur", sigmaX: 10, sigmaY: 15 });
  });

  test("maps the ends of a velocity blur's trail through any affine transform", () => {
    const velocity = (vector: number[]) => ({
      kind: "velocityBlur",
      from: vector.map((value) => -value),
      to: vector,
    });
    expect(programFilterInDeviceSpace(velocity([2, 1]), [0, -2, 8, 2, 0, 9, 0, 0, 1])).toEqual(
      velocity([-2, 4]),
    );
    // A shear keeps a one-dimensional blur exact.
    expect(programFilterInDeviceSpace(velocity([2, 1]), [1, 1, 0, 0, 1, 0, 0, 0, 1])).toEqual(
      velocity([3, 1]),
    );
    for (const matrix of [
      [1, 0, 0, 0, 1, 0, 0.01, 0, 1],
      [Number.MAX_VALUE, 0, 0, 0, Number.MAX_VALUE, 0, 0, 0, 1],
    ]) {
      expect(() => programFilterInDeviceSpace(velocity([2, 1]), matrix)).toThrow();
    }
  });

  test("maps backdrop lengths exactly where it can and through the nearest similarity elsewhere", () => {
    const at = [12, -7] as const;
    for (const similarity of [
      [1, 0, 0, 0, 1, 0, 0, 0, 1],
      [2, 0, 30, 0, 2, -4, 0, 0, 1],
      [0, -2, 8, 2, 0, 9, 0, 0, 1],
      [-2, 0, 5, 0, 2, 0, 0, 0, 1],
    ]) {
      expect(backdropSimilarity(similarity, at)?.map((value) => value + 0)).toEqual(similarity);
    }
    expect(backdropSimilarity([2, 0, 0, 0, 8, 0, 0, 0, 1], at)?.map((value) => value + 0)).toEqual([
      4, 0, -24, 0, 4, -28, 0, 0, 1,
    ]);
    const perspective = [1, 0, 0, 0, 1, 0, 0.01, 0, 1];
    expect(backdropSimilarity(perspective, [-100, 0])).toBeNull();
    // Native pins the same bits.
    expect(
      backdropSimilarity([1.3, -0.75, 61, 0.75, 1.3, 37, 0.0005, -0.0003, 1], [40, 25]),
    ).toEqual([
      1.2750681292645878, -0.7024338366812504, 59.644540499534166, 0.7024338366812504,
      1.2750681292645878, 38.29754823940689, 0, 0, 1,
    ]);

    const blur = { kind: "blur", sigmaX: 5, sigmaY: 5 };
    const noise = {
      kind: "noiseDisplacement",
      frequency: [0.05, 0.05],
      octaves: 2,
      seed: 3,
      scale: 7,
      turbulence: false,
    };
    const bounds = { x: 0, y: 0, width: 20, height: 0 };
    expect(
      backdropFiltersInDeviceSpace([blur, noise], [2, 0, 0, 0, 3, 0, 0, 0, 1], bounds),
    ).toEqual([{ kind: "blur", sigmaX: 10, sigmaY: 15 }, noise]);
    const sheared = backdropFiltersInDeviceSpace(
      [blur, noise],
      [1, 1, 0, 0, 1, 0, 0, 0, 1],
      bounds,
    );
    expect(Math.fround(Number(sheared[0]!.sigmaX))).toBe(5);
    expect(Math.fround(Number(sheared[0]!.sigmaY))).toBe(5);
    expect(sheared[1]).toEqual(noise);
    const scale = backdropSimilarity(perspective, [10, 0])![0]!;
    expect(backdropFiltersInDeviceSpace([blur], perspective, bounds)).toEqual([
      { kind: "blur", sigmaX: 5 * scale, sigmaY: 5 * scale },
    ]);
    expect(() =>
      backdropFiltersInDeviceSpace([blur], perspective, { ...bounds, x: -110 }),
    ).toThrow();
  });

  test("backdrop filters keep their shape under any transform", () => {
    const bounds = { x: 0, y: 0, width: 20, height: 10 };
    const lens = { kind: "lensDistortion", k1: 0.2, k2: 0 };
    const radial = { kind: "radialBlur", center: [10, 5], amount: 4 };
    const scaled = [2, 0, 3, 0, 2, 4, 0, 0, 1];
    const uneven = [2, 0, 0, 0, 3, 0, 0, 0, 1];
    const shear = [1, 1, 0, 0, 1, 0, 0, 0, 1];
    // A transform the filter takes stays; one it cannot take gives way to the similarity nearest
    // it at the bounds centre. Blur takes an uneven scale.
    expect(backdropFilterMatrix(lens, scaled, bounds)).toBe(scaled);
    expect(backdropFilterMatrix(radial, scaled, bounds)).toBe(scaled);
    for (const [filter, matrix] of [
      [lens, uneven],
      [lens, shear],
      [radial, uneven],
      [radial, shear],
    ] as const) {
      expect(backdropFilterMatrix(filter, matrix, bounds)).toEqual(
        backdropSimilarity(matrix, [10, 5])!,
      );
    }
    expect(backdropFilterMatrix({ kind: "blur", sigmaX: 2, sigmaY: 2 }, uneven, bounds)).toBe(
      uneven,
    );
  });

  test("rejects transform classes the scheduled axis-aligned kernel cannot represent", () => {
    const blur = { kind: "blur", sigmaX: 5, sigmaY: 5 };
    for (const matrix of [
      [1, 1, 0, 0, 1, 0, 0, 0, 1],
      [1, 0, 0, 0, 1, 0, 0.01, 0, 1],
      [1, 0, 1e12, 0, 1, 1e12, 1e-6, 0, 1],
      [Number.MAX_VALUE, 0, 0, 0, Number.MAX_VALUE, 0, 0, 0, 1],
    ]) {
      expect(() => programFilterInDeviceSpace(blur, matrix)).toThrow();
    }
  });
});

test("scheduled noise displacement keeps its noise and vectors in local units", async () => {
  const ck = await CanvasKitInit({ locateFile: () => wasmPath });
  const builtins = new CanvasKitBuiltinRuntime(ck);
  const [width, height] = [96, 64];
  const roi = { x: 40, y: 30, width, height };
  // A smooth ramp: a sample one pixel from the expected one stays within the tolerance, while
  // noise or vectors in other units land several pixels away. It fills the top left of a larger
  // pooled slot whose spare pixels are opaque magenta.
  const ramp = (x: number, y: number) => [(x + 0.5) / width, (y + 0.5) / height, 0.25, 1];
  const slot = [width + 24, height + 16] as const;
  const pixels = new Float32Array(slot[0] * slot[1] * 4);
  for (let y = 0; y < slot[1]; y++) {
    for (let x = 0; x < slot[0]; x++) {
      pixels.set(x < width && y < height ? ramp(x, y) : [1, 0, 1, 1], (y * slot[0] + x) * 4);
    }
  }
  const info = (w: number, h: number, alphaType = ck.AlphaType.Premul) => ({
    width: w,
    height: h,
    colorType: ck.ColorType.RGBA_F32,
    alphaType,
    colorSpace: ck.ColorSpace.SRGB,
  });
  const input = ck.MakeImage(info(...slot), new Uint8Array(pixels.buffer), slot[0] * 16)!;
  const [frequency, octaves, seed, scale] = [[0.05, 0.07] as const, 2, 3, 24];
  const filter = { kind: "noiseDisplacement", frequency, octaves, seed, scale, turbulence: false };
  const tables = new ProductEngine().perlin_noise_tables(seed);
  // The kernel's own noise, alone: CanvasKit's noise shader differs from Skia's left of and above
  // the lattice's origin.
  const noiseEffect = ck.RuntimeEffect.Make(
    noiseDisplacementSource.slice(0, noiseDisplacementSource.indexOf("half4 main(")) +
      "half4 main(float2 p) { float3 c = noise(p); return half4(half(c.x), half(c.y), 0.0, half(c.z)); }",
  )!;
  const lattice = ck.MakeImage(
    {
      width: 256,
      height: 5,
      colorType: ck.ColorType.RGBA_8888,
      alphaType: ck.AlphaType.Opaque,
      colorSpace: ck.ColorSpace.SRGB,
    },
    tables,
    256 * 4,
  )!;
  // A scheduled pass draws in device pixels on a canvas translated to its target origin.
  const render = (
    origin: readonly [number, number],
    size: readonly [number, number],
    draw: (canvas: Canvas) => void,
    alphaType = ck.AlphaType.Premul,
  ) => {
    const memory = ck.Malloc(Uint8Array, size[0] * size[1] * 16);
    const surface = ck.MakeRasterDirectSurface(info(...size), memory, size[0] * 16)!;
    try {
      const canvas = surface.getCanvas();
      canvas.clear(ck.TRANSPARENT);
      canvas.translate(-origin[0], -origin[1]);
      draw(canvas);
      return canvas.readPixels(0, 0, info(...size, alphaType)) as Float32Array;
    } finally {
      surface.delete();
      ck.Free(memory);
    }
  };
  // Device pixels 0..160 x 0..120 hold both targets below.
  const device = [160, 120] as const;
  try {
    for (const matrix of [
      [1, 0, 0, 0, 1, 0, 0, 0, 1],
      [2, 0, 61, 0, 2, 37, 0, 0, 1],
      [2, 0, 61, 0, 0.75, 37, 0, 0, 1],
      [-1.5, 0, 130, 0, 1.5, 37, 0, 0, 1],
      [1.3, -0.75, 61, 0.75, 1.3, 37, 0, 0, 1],
    ]) {
      // The noise drawn in the group's local space, read at device pixel centres.
      const noise = render(
        [0, 0],
        device,
        (canvas) => {
          canvas.concat(matrix);
          const image = ck.Shader.MakeColor(ck.TRANSPARENT, ck.ColorSpace.SRGB);
          const child = lattice.makeShaderOptions(
            ck.TileMode.Clamp,
            ck.TileMode.Clamp,
            ck.FilterMode.Nearest,
            ck.MipmapMode.None,
          );
          const uniforms = new Float32Array(19);
          uniforms.set([frequency[0], frequency[1], octaves, 0], 15);
          const shader = noiseEffect.makeShaderWithChildren(uniforms, [image, child]);
          const paint = new ck.Paint();
          try {
            paint.setShader(shader);
            canvas.drawPaint(paint);
          } finally {
            paint.delete();
            shader.delete();
            child.delete();
            image.delete();
          }
        },
        ck.AlphaType.Unpremul,
      );
      const [a, b, , d, e] = matrix as [number, number, number, number, number];
      let [compared, outside] = [0, 0];
      // The planner may start a target anywhere; the same device pixels come out.
      for (const [origin, size] of [
        [
          [28, 18],
          [120, 88],
        ],
        [
          [52, 42],
          [72, 40],
        ],
      ] as const) {
        const actual = render(origin, size, (canvas) =>
          drawNoiseDisplacement(
            ck,
            builtins,
            canvas,
            { image: input, owned: false, roi },
            filter,
            matrix,
            tables,
          ),
        );
        for (let y = 0; y < size[1]; y++) {
          for (let x = 0; x < size[0]; x++) {
            const [px, py] = [x + origin[0], y + origin[1]];
            const at = (py * device[0] + px) * 4;
            // `scale * (channel - 0.5)` local units, mapped into device pixels.
            const [dx, dy] = [noise[at]!, noise[at + 1]!].map(
              (value) => (Math.min(1, Math.max(0, value)) - 0.5) * scale,
            ) as [number, number];
            const sample = [
              Math.floor(px + 0.5 + a * dx + b * dy) - roi.x,
              Math.floor(py + 0.5 + d * dx + e * dy) - roi.y,
            ] as const;
            const pixel = Array.from(
              actual.subarray((y * size[0] + x) * 4, (y * size[0] + x) * 4 + 4),
            );
            if (
              sample[0] >= 2 &&
              sample[0] < width - 2 &&
              sample[1] >= 2 &&
              sample[1] < height - 2
            ) {
              const expected = ramp(...sample);
              if (!expected.every((value, channel) => Math.abs(pixel[channel]! - value) <= 0.02)) {
                throw new Error(
                  `${matrix} from ${origin} at ${px},${py}: ${pixel} against ${expected}`,
                );
              }
              compared++;
            } else if (
              sample[0] < -2 ||
              sample[0] >= width + 2 ||
              sample[1] < -2 ||
              sample[1] >= height + 2
            ) {
              // Beyond the input, including the slot's spare pixels.
              if (!pixel.every((value) => Math.abs(value) <= 1e-3)) {
                throw new Error(
                  `${matrix} from ${origin} at ${px},${py}: ${pixel} outside the input`,
                );
              }
              outside++;
            }
          }
        }
      }
      expect(compared).toBeGreaterThan(1000);
      expect(outside).toBeGreaterThan(100);
    }
  } finally {
    lattice.delete();
    noiseEffect.delete();
    input.delete();
    builtins.dispose();
  }
});
