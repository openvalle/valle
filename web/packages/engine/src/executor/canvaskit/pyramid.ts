import type { Canvas, CanvasKit, Image, Shader, Surface } from "canvaskit-wasm";
import { CanvasKitBuiltinRuntime, type BuiltinKernel } from "./builtin-runtime.ts";

interface Region { x: number; y: number; width: number; height: number }
export interface PyramidSurfacePool {
  /** GPU F16 attachments round natively; CPU test surfaces need explicit nearest-even stores. */
  readonly gpu?: boolean;
  transient(width: number, height: number): Surface;
  releaseTransient(surface: Surface): void;
}
interface Value { surface: Surface; image: Image; width: number; height: number }

/** Execute Engine's padded pyramid recipe without reading source pixels back to WASM. */
export function drawPyramidFilter(...args: Parameters<typeof pyramidStages>): void {
  for (const _ of pyramidStages(...args)) { /* preserve the synchronous reference API */ }
}

/** Same operations and stores, with owned intermediates retained across preview yields. */
export async function drawPyramidFilterCooperative(
  yieldToPresentation: () => Promise<void>, ...args: Parameters<typeof pyramidStages>
): Promise<void> {
  let yieldAt = performance.now();
  for (const _ of pyramidStages(...args)) {
    if (performance.now() - yieldAt >= 6) {
      await yieldToPresentation();
      yieldAt = performance.now();
    }
  }
}

/** Execute Engine's padded pyramid recipe without reading source pixels back to WASM. */
function* pyramidStages(
  ck: CanvasKit, builtins: CanvasKitBuiltinRuntime, pool: PyramidSurfacePool,
  canvas: Canvas, input: Image, inputRoi: Region, output: Region,
  kind: "glow" | "bloom", recipe: Float64Array,
): Generator<void, void> {
  const paint = new ck.Paint();
  paint.setBlendMode(ck.BlendMode.Src);
  const live = new Map<number, Value>();
  let base: Value | null = null;
  const release = (value: Value) => {
    value.image.delete();
    pool.releaseTransient(value.surface);
  };
  const sourceShader = (image: Image): Shader => {
    const shader = image.makeShaderOptions(ck.TileMode.Clamp, ck.TileMode.Clamp,
      ck.FilterMode.Nearest, ck.MipmapMode.None);
    if (!shader) throw new Error("GPU pyramid input shader allocation failed");
    return shader;
  };
  const emit = (width: number, height: number, draw: (target: Canvas) => void): Value => {
    const surface = pool.transient(width, height);
    try {
      const target = surface.getCanvas();
      target.save();
      try {
        target.clipRect(ck.XYWHRect(0, 0, width, height), ck.ClipOp.Intersect, false);
        target.clear(ck.TRANSPARENT);
        draw(target);
      } finally { target.restore(); }
      surface.flush();
      return { surface, image: surface.makeImageSnapshot([0, 0, width, height]), width, height };
    } catch (error) { pool.releaseTransient(surface); throw error; }
  };
  const drawKernel = (target: Canvas, kernel: BuiltinKernel, values: Value[], uniforms: number[]) => {
    const children: Shader[] = [];
    let shader: Shader | null = null;
    try {
      for (const value of values) children.push(sourceShader(value.image));
      shader = builtins.shader(kernel, uniforms, children, pool.gpu === false);
      paint.setShader(shader);
      target.drawPaint(paint);
    } finally { paint.setShader(null); shader?.delete(); children.forEach(child => child.delete()); }
  };
  const get = (id: number): Value => {
    const value = live.get(id);
    if (!value) throw new Error(`GPU pyramid resource ${id} is not live`);
    return value;
  };
  try {
    if (recipe.length === 0) {
      canvas.save();
      try {
        canvas.clipRect(ck.XYWHRect(output.x, output.y, output.width, output.height), ck.ClipOp.Intersect, false);
        canvas.drawImageRectOptions(input, ck.XYWHRect(0, 0, inputRoi.width, inputRoi.height),
          ck.XYWHRect(inputRoi.x, inputRoi.y, inputRoi.width, inputRoi.height),
          ck.FilterMode.Nearest, ck.MipmapMode.None, paint);
      } finally { canvas.restore(); }
      return;
    }
    if (recipe.length < 15 || !recipe.every(Number.isFinite)) throw new Error("Invalid GPU pyramid recipe");
    const width = recipe[2]!, height = recipe[3]!;
    base = emit(width, height, target => target.drawImageRectOptions(input,
      ck.XYWHRect(0, 0, inputRoi.width, inputRoi.height),
      ck.XYWHRect(recipe[4]!, recipe[5]!, inputRoi.width, inputRoi.height),
      ck.FilterMode.Nearest, ck.MipmapMode.None, paint));
    yield;
    live.set(0, emit(width, height, target => drawKernel(target,
      kind === "glow" ? "pyramidCoverage" : "pyramidThreshold", [base!],
      kind === "glow" ? [] : [recipe[9]!, recipe[10]!])));
    yield;
    let at = 15;
    for (let pass = 0; pass < recipe[14]!; pass++) {
      if (at + 8 > recipe.length) throw new Error("Truncated GPU pyramid pass");
      const [operation, source, fine, localWeight, id, w, h, retireCount] = recipe.subarray(at, at + 8);
      at += 8;
      const input = get(source!);
      let kernel: BuiltinKernel;
      let values: Value[];
      let uniforms: number[];
      if (operation === 0) {
        kernel = "pyramidDownsample"; values = [input]; uniforms = [input.width, input.height];
      } else if (operation === 1) {
        kernel = "pyramidBlur"; values = [input]; uniforms = [input.width, input.height, recipe[6]!];
      } else if (operation === 2) {
        kernel = "pyramidUpsample"; values = [input, fine! < 0 ? input : get(fine!)];
        uniforms = [input.width, input.height, w!, h!, recipe[6]!, localWeight!];
      } else throw new Error("Unknown GPU pyramid operation");
      live.set(id!, emit(w!, h!, target => drawKernel(target, kernel, values, uniforms)));
      if (at + retireCount! > recipe.length) throw new Error("Truncated GPU pyramid retirements");
      for (let i = 0; i < retireCount!; i++) {
        const retired = recipe[at++]!;
        release(get(retired)); live.delete(retired);
      }
      yield;
    }
    if (at !== recipe.length) throw new Error("Trailing GPU pyramid data");
    const halo = get(recipe[7]!);
    canvas.save();
    try {
      canvas.clipRect(ck.XYWHRect(output.x, output.y, output.width, output.height), ck.ClipOp.Intersect, false);
      canvas.translate(output.x + recipe[0]!, output.y + recipe[1]!);
      drawKernel(canvas, kind === "glow" ? "pyramidGlowComposite" : "pyramidComposite", [base, halo],
        kind === "glow" ? Array.from(recipe.subarray(9, 14)) : [recipe[13]!]);
    } finally { canvas.restore(); }
  } finally {
    for (const value of live.values()) release(value);
    if (base) release(base);
    paint.delete();
  }
}
