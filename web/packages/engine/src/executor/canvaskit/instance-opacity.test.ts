import { expect, test } from "bun:test";
import type { CanvasKit } from "canvaskit-wasm";

import { CanvasKitBuiltinRuntime } from "./builtin-runtime.ts";
import type { DrawProgramWire } from "./draw-program.ts";
import { drawProgramNode } from "./executor.ts";
import { instanceOpacityBounds } from "./program-bounds.ts";

const { default: CanvasKitInit } = (await import("canvaskit-wasm")) as unknown as {
  default: (options: { locateFile(file: string): string }) => Promise<CanvasKit>;
};
const identity = [1, 0, 0, 0, 1, 0, 0, 0, 1];

test("rectangle-equivalent instance fills skip layers and preserve isolated F16 pixels", async () => {
  const CK = await CanvasKitInit({
    locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm", import.meta.dir),
  });
  const builtins = new CanvasKitBuiltinRuntime(CK);
  const info = {
    width: 64,
    height: 64,
    colorType: CK.ColorType.RGBA_F16,
    alphaType: CK.AlphaType.Premul,
    colorSpace: CK.ColorSpace.SRGB,
  };
  const rect = { x: 0, y: 0, width: 1, height: 1 };
  for (const [shape, expectedLayers, fill, opacity, strokeWidth, strokeAlpha] of [
    [{ kind: "rect" }, 0, true, 0.75, 0, 0],
    [
      {
        kind: "roundRect",
        value: {
          rect,
          radii: [
            [0, 0],
            [0, 0],
            [0, 0],
            [0, 0],
          ],
        },
      },
      0,
      true,
      0.75,
      0,
      0,
    ],
    [{ kind: "path", value: 0 }, 0, true, 0.75, 1, 0],
    [{ kind: "circle" }, 2, true, 0.75, 0, 0],
    [
      {
        kind: "roundRect",
        value: {
          rect,
          radii: [
            [0.2, 0.2],
            [0.2, 0.2],
            [0.2, 0.2],
            [0.2, 0.2],
          ],
        },
      },
      2,
      true,
      0.75,
      0,
      0,
    ],
    [{ kind: "path", value: 0 }, 2, true, 0.75, 0.25, 0.5],
    [{ kind: "path", value: 0 }, 2, false, 0.75, 0.25, 0.5],
    // particles() already carries alpha in its color, with no independent opacity layer.
    [{ kind: "circle" }, 0, true, 1, 0, 0],
  ] as const) {
    const count = 2;
    const bytes = new Uint8Array(count * 72),
      view = new DataView(bytes.buffer);
    for (let row = 0; row < count; row++) {
      for (const [index, value] of [24, 0, 0, 24, 12 + row * 8, 12 + row * 4].entries())
        view.setFloat64(row * 48 + index * 8, value, true);
      for (const [index, value] of [0.5, 0.125, 0.25, 0.5].entries())
        view.setFloat32(count * 48 + row * 16 + index * 4, value, true);
      view.setFloat32(count * 64 + row * 4, opacity, true);
      view.setFloat32(count * 68 + row * 4, strokeWidth, true);
    }
    const strokeBytes = new Uint8Array(count * 16),
      strokeView = new DataView(strokeBytes.buffer);
    for (let row = 0; row < count; row++)
      for (const [index, value] of [0.125, 0.25, 0.5, strokeAlpha].entries())
        strokeView.setFloat32(row * 16 + index * 4, strokeAlpha === 0 ? 0 : value, true);
    const empty = { count: 0, bytes: new Uint8Array(), view: new DataView(new ArrayBuffer(0)) };
    const node = {
      shape,
      instances: { start: 0, count },
      strokeColors: { start: 0, count },
      dashOffsets: null,
      pathStyle:
        shape.kind === "path"
          ? { fill, cap: "butt", join: "miter", miterLimit: 4, dash: [], dashOffset: 0 }
          : null,
    };
    const program: DrawProgramWire = {
      roots: [0],
      nodes: [{ kind: "instanceBatch", value: node }],
      paths: [
        {
          verbs: ["moveTo", "lineTo", "lineTo", "lineTo", "close"],
          points: [
            [0, 0],
            [1, 0],
            [1, 1],
            [0, 1],
          ],
        },
      ],
      paints: [],
      viewport: { x: 0, y: 0, width: 64, height: 64 },
      batchInstances: { count, bytes, view },
      strokeColors: { count, bytes: strokeBytes, view: strokeView },
      dashOffsets: empty,
      requirements: {},
    };
    const admitted: Parameters<typeof drawProgramNode>[3] = {
      draw: program,
      plan: {},
      directRaster: null,
      textures: new Map(),
      fonts: new Map(),
      shaders: new Map(),
      scenes: new Map(),
    };
    const memory = [CK.Malloc(Uint8Array, 64 * 64 * 8), CK.Malloc(Uint8Array, 64 * 64 * 8)];
    const surfaces = memory.map((mem) => CK.MakeRasterDirectSurface(info, mem, 64 * 8)!);
    const layer = new CK.Paint();
    try {
      for (const surface of surfaces) surface.getCanvas().clear(CK.Color4f(0.1, 0.2, 0.3, 0.7));
      const canvas = surfaces[0]!.getCanvas();
      const saveLayer = canvas.saveLayer.bind(canvas);
      let layers = 0;
      canvas.saveLayer = (...args) => {
        layers++;
        return saveLayer(...args);
      };
      drawProgramNode(CK, builtins, canvas, admitted, 0, identity);
      expect(layers).toBe(expectedLayers);
      layer.setAlphaf(opacity);
      node.instances.count = 1;
      const reference = surfaces[1]!.getCanvas();
      for (let row = 0; row < count; row++) {
        node.instances.start = row;
        view.setFloat32(count * 64 + row * 4, 1, true);
        const transform: [number, number, number, number, number, number] = [
          view.getFloat64(row * 48, true),
          view.getFloat64(row * 48 + 8, true),
          view.getFloat64(row * 48 + 16, true),
          view.getFloat64(row * 48 + 24, true),
          view.getFloat64(row * 48 + 32, true),
          view.getFloat64(row * 48 + 40, true),
        ];
        const bounds = instanceOpacityBounds(
          program,
          shape,
          node.pathStyle,
          strokeWidth,
          transform,
        );
        if (opacity < 1) {
          reference.saveLayer(
            layer,
            bounds === null
              ? undefined
              : CK.XYWHRect(bounds.x, bounds.y, bounds.width, bounds.height),
          );
        } else reference.save();
        drawProgramNode(CK, builtins, reference, admitted, 0, identity);
        reference.restore();
      }
      expect(new Uint8Array(memory[0]!.toTypedArray())).toEqual(
        new Uint8Array(memory[1]!.toTypedArray()),
      );
    } finally {
      layer.delete();
      for (const surface of surfaces) surface.delete();
      for (const mem of memory) CK.Free(mem);
    }
  }
});
