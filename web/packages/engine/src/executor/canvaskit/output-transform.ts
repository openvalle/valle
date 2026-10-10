import type { Canvas, CanvasKit, Image } from "canvaskit-wasm";

export interface SrgbPreviewCpuKernel {
  transform_srgb_preview_pixels(pixels: Float32Array, opaque: boolean): void;
  transform_srgb_preview_f16?(pixels: Uint8Array, opaque: boolean): void;
}

const SCRATCH_ROWS = 64;
export function srgbPreviewScratchBytes(width: number, height: number, f16 = false): bigint {
  return BigInt(width) * BigInt(Math.min(height, SCRATCH_ROWS)) * (f16 ? 8n : 16n);
}

/** CPU-only SDR output. Float pixels cross the boundary without color-space conversion;
 * writing back into the existing F16 surface keeps the same intermediate quantization.
 * Callers must admit a full-frame, untransformed destination and budget the scratch buffers.
 * Coverage outputs may opt in only after every source pixel proves full coverage. A rejected
 * probe leaves the destination untouched, preserving SkSL's low-alpha quantization behavior.
 */
export function drawSrgbPreviewCpu(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  input: Image,
  kernel: SrgbPreviewCpuKernel,
  opaque: boolean,
  requireFullCoverage = false,
): boolean {
  const width = input.width(), height = input.height();
  // An explicit destination also avoids CanvasKit 0.42's default RGBA_F32 reader treating
  // its byte length as a float count. The byte view can be written back without another copy.
  const rows = Math.min(height, SCRATCH_ROWS);
  const f16 = typeof kernel.transform_srgb_preview_f16 === "function";
  const pixelBytes = f16 ? 8 : 16;
  const colorType = f16 ? CanvasKit.ColorType.RGBA_F16 : CanvasKit.ColorType.RGBA_F32;
  const memory = CanvasKit.Malloc(Uint8Array, width * rows * pixelBytes);
  try {
    const readRows = (y: number, count: number) => {
      const bytes = input.readPixels(0, y, {
        width, height: count,
        colorType,
        alphaType: CanvasKit.AlphaType.Premul,
        colorSpace: CanvasKit.ColorSpace.SRGB,
      }, memory, width * pixelBytes);
      if (!bytes) throw new Error("cannot read CPU output float pixels");
      return new Uint8Array(bytes.buffer, bytes.byteOffset, width * count * pixelBytes);
    };
    // Check the entire immutable input before any write, including the final short stripe.
    if (requireFullCoverage) {
      for (let y = 0; y < height; y += rows) {
        const pixels = readRows(y, Math.min(rows, height - y));
        if (f16) {
          const halves = new Uint16Array(pixels.buffer, pixels.byteOffset, pixels.byteLength / 2);
          for (let alpha = 3; alpha < halves.length; alpha += 4) {
            if (halves[alpha] !== 0x3c00) return false;
          }
        } else {
          const floats = new Float32Array(pixels.buffer, pixels.byteOffset, pixels.byteLength / 4);
          for (let alpha = 3; alpha < floats.length; alpha += 4) {
            if (floats[alpha] !== 1) return false;
          }
        }
      }
    }
    for (let y = 0; y < height; y += rows) {
      const count = Math.min(rows, height - y);
      const pixels = readRows(y, count);
      if (f16) kernel.transform_srgb_preview_f16!(pixels, opaque);
      else kernel.transform_srgb_preview_pixels(new Float32Array(pixels.buffer, pixels.byteOffset, pixels.byteLength / 4), opaque);
      // MallocObj.subarray keeps CanvasKit's direct-memory marker, including short stripes.
      const bytes = memory.subarray(0, pixels.byteLength) as Uint8Array;
      if (!canvas.writePixels(bytes, width, count, 0, y,
        CanvasKit.AlphaType.Premul, colorType, CanvasKit.ColorSpace.SRGB)) {
        throw new Error("cannot write CPU output float pixels");
      }
    }
    return true;
  } finally { CanvasKit.Free(memory); }
}
