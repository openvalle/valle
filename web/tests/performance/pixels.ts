import type { RgbaImage } from "../parity/lib/metrics.ts";

export interface PixelRule { rgb: number; alpha: number }
export interface Region { name: string; x: number; y: number; width: number; height: number }

/** Straight sRGB RGBA input; compare premultiplied RGB, independent alpha and
 * composites over black/white. No global average or unregistered edge exclusions. */
export function compareFrames(reference: RgbaImage, actual: RgbaImage, rule: PixelRule, region?: Region) {
  validate(reference); validate(actual);
  if (reference.width !== actual.width || reference.height !== actual.height) throw new Error("frame dimensions differ");
  if (![rule.rgb, rule.alpha].every(n => Number.isFinite(n) && n >= 0)) throw new Error("invalid pixel tolerance");
  const roi = region ?? { name: "full", x: 0, y: 0, width: reference.width, height: reference.height };
  if (![roi.x, roi.y, roi.width, roi.height].every(Number.isSafeInteger) || roi.x < 0 || roi.y < 0
    || roi.width < 1 || roi.height < 1 || roi.x + roi.width > reference.width || roi.y + roi.height > reference.height) throw new Error("invalid pixel region");
  let rgbMax = 0, alphaMax = 0, whiteMax = 0, failedPixels = 0;
  const diff = new Uint8Array(reference.data.length);
  for (let y = roi.y; y < roi.y + roi.height; y++) for (let x = roi.x; x < roi.x + roi.width; x++) {
    const i = (y * reference.width + x) * 4;
    const a = reference.data[i + 3]!, b = actual.data[i + 3]!;
    let rgb = 0, white = 0;
    for (let c = 0; c < 3; c++) {
      const av = reference.data[i + c]! * a / 255, bv = actual.data[i + c]! * b / 255;
      rgb = Math.max(rgb, Math.abs(av - bv));
      white = Math.max(white, Math.abs((av - bv) - (a - b)));
    }
    const alpha = Math.abs(a - b);
    rgbMax = Math.max(rgbMax, rgb); alphaMax = Math.max(alphaMax, alpha); whiteMax = Math.max(whiteMax, white);
    const fail = rgb > rule.rgb || alpha > rule.alpha || white > rule.rgb;
    if (fail) failedPixels++;
    diff[i] = fail ? 255 : Math.min(255, Math.ceil(rgb * 16));
    diff[i + 1] = Math.min(255, Math.ceil(alpha * 16));
    diff[i + 2] = Math.min(255, Math.ceil(white * 16)); diff[i + 3] = 255;
  }
  return { pass: failedPixels === 0, region: roi.name, rgbMax, alphaMax, blackMax: rgbMax, whiteMax, failedPixels,
    diff: { width: reference.width, height: reference.height, data: diff } };
}

/** Float intermediates stay in their original value range (including HDR > 1). */
export function compareFloatPixels(reference: ArrayLike<number>, actual: ArrayLike<number>, tolerance: number) {
  if (reference.length !== actual.length || !Number.isFinite(tolerance) || tolerance < 0) throw new Error("invalid float comparison");
  let max = 0;
  for (let i = 0; i < reference.length; i++) {
    const a = reference[i]!, b = actual[i]!;
    if (!Number.isFinite(a) || !Number.isFinite(b)) return { pass: false, max: Infinity, invalidAt: i };
    max = Math.max(max, Math.abs(a - b));
  }
  return { pass: max <= tolerance, max, invalidAt: null };
}

export function crop(image: RgbaImage, x: number, y: number, width: number, height: number): RgbaImage {
  if (x < 0 || y < 0 || x + width > image.width || y + height > image.height) throw new Error("invalid crop");
  const data = new Uint8Array(width * height * 4);
  for (let row = 0; row < height; row++) data.set(image.data.subarray(((y + row) * image.width + x) * 4,
    ((y + row) * image.width + x + width) * 4), row * width * 4);
  return { width, height, data };
}

function validate(image: RgbaImage) {
  if (!Number.isSafeInteger(image.width) || !Number.isSafeInteger(image.height) || image.width < 1 || image.height < 1
    || image.data.length !== image.width * image.height * 4) throw new Error("invalid RGBA frame");
}
