export interface RgbaImage {
  width: number;
  height: number;
  data: Uint8Array | Uint8ClampedArray;
}

export interface PixelMetricOptions {
  diffTolerance?: number;
}

export interface PixelMetrics {
  psnr: number;
  mse: number;
  meanAbs: number;
  maxAbs: number;
  diffPixels: number;
  diffTolerance: number;
  maxDeltaE76: number;
  meanDeltaE76: number;
}

export function comparePngs(
  native: RgbaImage,
  web: RgbaImage,
  options: PixelMetricOptions = {},
): PixelMetrics {
  if (native.width !== web.width || native.height !== web.height) {
    throw new Error(
      `PNG dimensions differ: native ${native.width}x${native.height}, web ${web.width}x${web.height}`,
    );
  }
  if (native.data.length !== web.data.length) {
    throw new Error(`RGBA byte lengths differ: native ${native.data.length}, web ${web.data.length}`);
  }
  const premul = (data: Uint8Array | Uint8ClampedArray, index: number, channel: number): number =>
    Math.round((data[index + channel]! * data[index + 3]!) / 255);

  let sumSq = 0;
  let sumAbs = 0;
  let maxAbs = 0;
  const tolerance = Number.isFinite(options.diffTolerance) ? options.diffTolerance! : 2;
  let diffPixels = 0;
  let maxDeltaE = 0;
  let sumDeltaE = 0;
  const nativePremul = new Uint8ClampedArray(native.data.length);
  const webPremul = new Uint8ClampedArray(web.data.length);
  for (let index = 0; index < native.data.length; index += 4) {
    let worst = 0;
    for (let channel = 0; channel < 3; channel += 1) {
      const nativeValue = premul(native.data, index, channel);
      const webValue = premul(web.data, index, channel);
      nativePremul[index + channel] = nativeValue;
      webPremul[index + channel] = webValue;
      const difference = nativeValue - webValue;
      const absolute = Math.abs(difference);
      sumSq += difference * difference;
      sumAbs += absolute;
      maxAbs = Math.max(maxAbs, absolute);
      worst = Math.max(worst, absolute);
    }
    nativePremul[index + 3] = 255;
    webPremul[index + 3] = 255;
    if (worst > tolerance) {
      diffPixels += 1;
      const delta = deltaE76(nativePremul, webPremul, index);
      sumDeltaE += delta;
      maxDeltaE = Math.max(maxDeltaE, delta);
    }
  }
  const pixels = native.width * native.height;
  const channels = pixels * 3;
  const mse = sumSq / channels;
  const psnr = mse === 0 ? 99 : 10 * Math.log10((255 * 255) / mse);
  return {
    psnr: roundMetric(psnr),
    mse: roundMetric(mse),
    meanAbs: roundMetric(sumAbs / channels),
    maxAbs,
    diffPixels,
    diffTolerance: tolerance,
    maxDeltaE76: roundMetric(maxDeltaE),
    meanDeltaE76: roundMetric(sumDeltaE / pixels),
  };
}

const SRGB_TO_LINEAR = Float64Array.from({ length: 256 }, (_, index) => {
  const component = index / 255;
  return component <= 0.04045 ? component / 12.92 : ((component + 0.055) / 1.055) ** 2.4;
});

function labF(value: number): number {
  return value > 216 / 24389 ? Math.cbrt(value) : ((24389 / 27) * value + 16) / 116;
}

function srgbToLab(
  data: Uint8Array | Uint8ClampedArray,
  index: number,
  out: Float64Array,
): void {
  const red = SRGB_TO_LINEAR[data[index]!]!;
  const green = SRGB_TO_LINEAR[data[index + 1]!]!;
  const blue = SRGB_TO_LINEAR[data[index + 2]!]!;
  const fx = labF((0.4124564 * red + 0.3575761 * green + 0.1804375 * blue) / 0.95047);
  const fy = labF(0.2126729 * red + 0.7151522 * green + 0.072175 * blue);
  const fz = labF((0.0193339 * red + 0.119192 * green + 0.9503041 * blue) / 1.08883);
  out[0] = 116 * fy - 16;
  out[1] = 500 * (fx - fy);
  out[2] = 200 * (fy - fz);
}

const LAB_A = new Float64Array(3);
const LAB_B = new Float64Array(3);

function deltaE76(
  nativeData: Uint8Array | Uint8ClampedArray,
  webData: Uint8Array | Uint8ClampedArray,
  index: number,
): number {
  srgbToLab(nativeData, index, LAB_A);
  srgbToLab(webData, index, LAB_B);
  const lightness = LAB_A[0]! - LAB_B[0]!;
  const a = LAB_A[1]! - LAB_B[1]!;
  const b = LAB_A[2]! - LAB_B[2]!;
  return Math.sqrt(lightness * lightness + a * a + b * b);
}

export function countNonBlackPixels(rgba: Uint8Array | Uint8ClampedArray): number {
  let count = 0;
  for (let index = 0; index < rgba.length; index += 4) {
    if (rgba[index] !== 0 || rgba[index + 1] !== 0 || rgba[index + 2] !== 0) count += 1;
  }
  return count;
}

function roundMetric(value: number): number {
  return Number(value.toFixed(6));
}
