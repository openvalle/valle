import type { Canvas, CanvasKit, Image, Shader, Surface, TileMode } from "canvaskit-wasm";
import { CanvasKitBuiltinRuntime, type BuiltinKernel } from "./builtin-runtime.ts";

interface Region { x: number; y: number; width: number; height: number }
export interface PyramidSurfacePool {
  /** GPU F16 attachments round natively; CPU test surfaces need explicit nearest-even stores. */
  readonly gpu?: boolean;
  /** Intermediate snapshots share the Ganesh context; submit at the frame boundary. */
  readonly deferGpuFlush?: boolean;
  transient(width: number, height: number): Surface;
  /** A surface at least this large, for consumers that read only its leading extent. */
  boundedTransient?(width: number, height: number): Surface;
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
  const imageShader = (image: Image, tile: TileMode, x = 0, y = 0): Shader => {
    const shader = image.makeShaderOptions(tile, tile, ck.FilterMode.Nearest, ck.MipmapMode.None,
      x === 0 && y === 0 ? undefined : ck.Matrix.translated(x, y));
    if (!shader) throw new Error("GPU pyramid input shader allocation failed");
    return shader;
  };
  const sourceShader = (value: Value) => () => imageShader(value.image, ck.TileMode.Clamp);
  // Every pass writes and reads only [0, width) x [0, height): the resampling kernels return zero
  // outside `sourceSize` and the point kernels read their own pixel. A larger backing therefore
  // serves an extent exactly, and its full snapshot needs no subset copy. A pass that covers only
  // part of the padded region places its surface at `x, y` so kernels still see region coordinates.
  const emit = (width: number, height: number, draw: (target: Canvas) => void, x = 0, y = 0): Value => {
    const surface = pool.boundedTransient?.(width, height) ?? pool.transient(width, height);
    try {
      const target = surface.getCanvas();
      target.save();
      try {
        target.clipRect(ck.XYWHRect(0, 0, width, height), ck.ClipOp.Intersect, false);
        target.clear(ck.TRANSPARENT);
        target.translate(-x, -y);
        draw(target);
      } finally { target.restore(); }
      if (pool.gpu === false || !pool.deferGpuFlush) surface.flush();
      return { surface, image: surface.makeImageSnapshot(), width, height };
    } catch (error) { pool.releaseTransient(surface); throw error; }
  };
  const drawKernel = (target: Canvas, kernel: BuiltinKernel, sources: (() => Shader)[], uniforms: number[]) => {
    const children: Shader[] = [];
    let shader: Shader | null = null;
    try {
      for (const source of sources) children.push(source());
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
    // The part of the output inside the padded region, in region coordinates. Output pixels
    // outside it are farther than the pyramid support from every source pixel: transparent.
    const haloX = Math.max(0, -recipe[0]!), haloY = Math.max(0, -recipe[1]!);
    const haloWidth = Math.min(width, output.width - recipe[0]!) - haloX;
    const haloHeight = Math.min(height, output.height - recipe[1]!) - haloY;
    if (haloWidth <= 0 || haloHeight <= 0) throw new Error("GPU pyramid region misses its output");
    // A snapshot of exactly its ROI reads transparent past its edges, like the zero-padded base, so
    // kernels sample it in place. A larger backing holds pixels beyond the ROI and is copied first.
    if (input.width() !== inputRoi.width || input.height() !== inputRoi.height) {
      base = emit(width, height, target => target.drawImageRectOptions(input,
        ck.XYWHRect(0, 0, inputRoi.width, inputRoi.height),
        ck.XYWHRect(recipe[4]!, recipe[5]!, inputRoi.width, inputRoi.height),
        ck.FilterMode.Nearest, ck.MipmapMode.None, paint));
      yield;
    }
    const baseShader = base ? sourceShader(base) : () => imageShader(input, ck.TileMode.Decal, recipe[4]!, recipe[5]!);
    // The glow's first level is the base alpha, which its first downsample reads directly.
    const coverageInPlace = kind === "glow";
    if (!coverageInPlace) {
      live.set(0, emit(width, height, target => drawKernel(target, "pyramidThreshold", [baseShader],
        [recipe[9]!, recipe[10]!])));
      yield;
    }
    // The first coarse row that the halo's horizontal factor holds, and the coarse level's height.
    let rowsY = 0, coarseHeight = 0;
    let at = 15;
    for (let pass = 0; pass < recipe[14]!; pass++) {
      if (at + 8 > recipe.length) throw new Error("Truncated GPU pyramid pass");
      const [operation, source, fine, localWeight, id, w, h, retireCount] = recipe.subarray(at, at + 8);
      at += 8;
      if (id === recipe[7]!) {
        // Only the composite reads the halo, and only where the output meets the region. Its tent
        // is separable: this pass applies the horizontal factor to the coarse rows those output
        // pixels read, padded by a row against rounding of the taps; the composite completes it.
        if (operation !== 2 || localWeight !== 0) throw new Error("Invalid GPU pyramid halo pass");
        const coarse = get(source!), scale = coarse.height / height, tap = 0.5 * recipe[6]!;
        rowsY = Math.max(0, Math.floor((haloY + 0.5) * scale - 0.5 - tap) - 1);
        const rowsEnd = Math.min(coarse.height, Math.floor((haloY + haloHeight - 0.5) * scale - 0.5 + tap) + 3);
        coarseHeight = coarse.height;
        live.set(id!, emit(haloWidth, rowsEnd - rowsY, target => drawKernel(target, "pyramidUpsampleRows",
          [sourceShader(coarse)], [coarse.width, coarse.height, width, recipe[6]!]), haloX, rowsY));
      } else {
        let kernel: BuiltinKernel;
        let sources: (() => Shader)[];
        let uniforms: number[];
        if (operation === 0 && source === 0 && coverageInPlace) {
          kernel = "pyramidCoverageDownsample"; sources = [baseShader]; uniforms = [width, height];
        } else {
          const input = get(source!);
          if (operation === 0) {
            kernel = "pyramidDownsample"; sources = [sourceShader(input)]; uniforms = [input.width, input.height];
          } else if (operation === 1) {
            kernel = "pyramidBlur"; sources = [sourceShader(input)]; uniforms = [input.width, input.height, recipe[6]!];
          } else if (operation === 2) {
            kernel = "pyramidUpsample"; sources = [sourceShader(input), sourceShader(fine! < 0 ? input : get(fine!))];
            uniforms = [input.width, input.height, w!, h!, recipe[6]!, localWeight!];
          } else throw new Error("Unknown GPU pyramid operation");
        }
        live.set(id!, emit(w!, h!, target => drawKernel(target, kernel, sources, uniforms)));
      }
      if (at + retireCount! > recipe.length) throw new Error("Truncated GPU pyramid retirements");
      for (let i = 0; i < retireCount!; i++) {
        const retired = recipe[at++]!;
        if (retired === 0 && coverageInPlace) continue;
        release(get(retired)); live.delete(retired);
      }
      yield;
    }
    if (at !== recipe.length) throw new Error("Trailing GPU pyramid data");
    const halo = get(recipe[7]!);
    canvas.save();
    try {
      canvas.clipRect(ck.XYWHRect(output.x, output.y, output.width, output.height), ck.ClipOp.Intersect, false);
      if (haloWidth < output.width || haloHeight < output.height) canvas.clear(ck.TRANSPARENT);
      canvas.translate(output.x + recipe[0]!, output.y + recipe[1]!);
      canvas.clipRect(ck.XYWHRect(haloX, haloY, haloWidth, haloHeight), ck.ClipOp.Intersect, false);
      drawKernel(canvas, kind === "glow" ? "pyramidGlowUpsampleComposite" : "pyramidUpsampleComposite",
        [baseShader, () => imageShader(halo.image, ck.TileMode.Clamp, haloX, rowsY)],
        [coarseHeight, height, recipe[6]!, ...(kind === "glow" ? recipe.subarray(9, 14) : [recipe[13]!])]);
    } finally { canvas.restore(); }
  } finally {
    for (const value of live.values()) release(value);
    if (base) release(base);
    paint.delete();
  }
}
