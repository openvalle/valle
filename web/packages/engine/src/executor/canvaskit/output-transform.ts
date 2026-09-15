import type { Canvas, CanvasKit, Image } from "canvaskit-wasm";

export interface SrgbPreviewCpuKernel {
  transform_srgb_preview_pixels(pixels: Float32Array, opaque: boolean): void;
}

/** CPU-only SDR output. Float pixels cross the boundary without color-space conversion;
 * writing back into the existing F16 surface keeps the same intermediate quantization.
 * Callers must admit a full-frame, untransformed destination and budget the scratch buffers.
 */
export function drawSrgbPreviewCpu(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  input: Image,
  kernel: SrgbPreviewCpuKernel,
  opaque: boolean,
): void {
  const width = input.width(), height = input.height();
  // An explicit destination also avoids CanvasKit 0.42's default RGBA_F32 reader treating
  // its byte length as a float count. The byte view can be written back without another copy.
  const memory = CanvasKit.Malloc(Uint8Array, width * height * 16);
  try {
    const bytes = input.readPixels(0, 0, {
      width, height,
      colorType: CanvasKit.ColorType.RGBA_F32,
      alphaType: CanvasKit.AlphaType.Premul,
      colorSpace: CanvasKit.ColorSpace.SRGB,
    }, memory, width * 16);
    if (!bytes) throw new Error("cannot read CPU output float pixels");
    const pixels = new Float32Array(bytes.buffer, bytes.byteOffset, width * height * 4);
    kernel.transform_srgb_preview_pixels(pixels, opaque);
    if (!canvas.writePixels(memory.toTypedArray() as Uint8Array, width, height, 0, 0,
      CanvasKit.AlphaType.Premul, CanvasKit.ColorType.RGBA_F32, CanvasKit.ColorSpace.SRGB)) {
      throw new Error("cannot write CPU output float pixels");
    }
  } finally { CanvasKit.Free(memory); }
}
