import type { Canvas, CanvasKit, Image, Shader } from "canvaskit-wasm";
import { CanvasKitBuiltinRuntime } from "./builtin-runtime.ts";

interface Region {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Warp the working F16 image in place on the GPU, retaining logical decal bounds. */
export function drawLensDistortion(
  ck: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  input: Image,
  inputRoi: Region,
  output: Region,
  frame: readonly number[],
  k1: number,
  k2: number,
): void {
  // Engine admits the filter before execution; retain checks for direct helper callers too.
  if (
    frame.length !== 4 ||
    !frame.every((v) => Number.isFinite(v) && Math.abs(v) <= 10_000_000) ||
    frame[2]! <= 0 ||
    frame[3]! <= 0 ||
    !Number.isFinite(k1) ||
    !Number.isFinite(k2) ||
    Math.abs(k1) > 0.5 ||
    Math.abs(k2) > 0.5
  )
    throw new Error("Invalid lens distortion parameters");
  const source = input.makeShaderOptions(
    ck.TileMode.Clamp,
    ck.TileMode.Clamp,
    ck.FilterMode.Nearest,
    ck.MipmapMode.None,
  );
  if (!source) throw new Error("GPU lens input shader allocation failed");
  const paint = new ck.Paint();
  let shader: Shader | null = null;
  canvas.save();
  try {
    // The CPU kernel admits f32 parameters. Keep their exact values in both backends.
    shader = builtins.shader(
      "lensDistortion",
      [
        inputRoi.width,
        inputRoi.height,
        inputRoi.x - output.x,
        inputRoi.y - output.y,
        output.x,
        output.y,
        ...frame,
        k1,
        k2,
      ],
      [source],
    );
    paint.setBlendMode(ck.BlendMode.Src);
    paint.setShader(shader);
    canvas.translate(output.x, output.y);
    canvas.drawPaint(paint);
  } finally {
    canvas.restore();
    paint.delete();
    shader?.delete();
    source.delete();
  }
}
