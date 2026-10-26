import type { CanvasKit, GrDirectContext, Image, Surface } from "canvaskit-wasm";

/** Probe the actual F16 target, including HDR and premultiplied alpha. Extension presence alone
 * does not prove CanvasKit can use that format on this context. Called once when a target opens.
 */
export function supportsWorkingF16(CanvasKit: CanvasKit, context: GrDirectContext): boolean {
  let surface: Surface | null = null;
  let image: Image | null = null;
  try {
    const info = {
      width: 1,
      height: 1,
      colorType: CanvasKit.ColorType.RGBA_F16,
      alphaType: CanvasKit.AlphaType.Premul,
      colorSpace: CanvasKit.ColorSpace.SRGB,
    };
    surface = CanvasKit.MakeRenderTarget(context, info);
    if (!surface) return false;
    surface.getCanvas().clear(new Float32Array([0.25, 0.5, 4, 0.5]));
    surface.flush();
    image = surface.makeImageSnapshot();
    if (image.getImageInfo().colorType !== CanvasKit.ColorType.RGBA_F16) return false;
    const pixels = image.readPixels(0, 0, {
      ...info,
      colorType: CanvasKit.ColorType.RGBA_F32,
    });
    return (
      pixels instanceof Float32Array &&
      [0.125, 0.25, 2, 0.5].every(
        (value, channel) => Math.abs(pixels[channel]! - value) <= 1 / 1024,
      )
    );
  } catch {
    return false;
  } finally {
    image?.delete();
    surface?.delete();
  }
}
