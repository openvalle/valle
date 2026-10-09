/** MP4 delivery settings. Omitted values retain the admitted timeline's size and frame rate. */
export interface BrowserVideoExportSettings {
  /** Output dimensions in pixels; provide both. The source fits inside them without stretching. */
  width?: number;
  height?: number;
  /** Output frames per second. Source frames are repeated/dropped without changing playback speed. */
  frameRate?: number;
  /** Target H.264 bitrate in bits per second. Omit for automatic high quality. */
  videoBitrate?: number;
  includeAudio?: boolean;
  /** Target AAC bitrate in bits per second; defaults to 192000. */
  audioBitrate?: number;
}

interface ExportRenderInfo {
  canvasWidth: number;
  canvasHeight: number;
  frameRate: string;
  frameCount: number;
  sampleRate: number;
  sampleCount: number;
}

function integer(value: number, label: string, min: number, max: number): number {
  if (!Number.isSafeInteger(value) || value < min || value > max) {
    throw new RangeError(`${label} must be an integer between ${min} and ${max}.`);
  }
  return value;
}

export function resolveExportPlan(info: ExportRenderInfo, settings: BrowserVideoExportSettings) {
  if ((settings.width === undefined) !== (settings.height === undefined)) {
    throw new RangeError("Provide both export width and height.");
  }
  const width = integer(settings.width ?? info.canvasWidth, "Export width", 2, 8192);
  const height = integer(settings.height ?? info.canvasHeight, "Export height", 2, 8192);
  if (width % 2 || height % 2) throw new RangeError("MP4 export requires even canvas width and height.");
  if (settings.videoBitrate !== undefined) integer(settings.videoBitrate, "Video bitrate", 100_000, 200_000_000);
  const audioBitrate = integer(settings.audioBitrate ?? 192_000, "Audio bitrate", 32_000, 512_000);
  if (settings.includeAudio !== undefined && typeof settings.includeAudio !== "boolean") {
    throw new TypeError("Include audio must be a boolean.");
  }
  const [sourceNumerator, sourceDenominator] = info.frameRate.split("/").map(BigInt);
  const numerator = settings.frameRate === undefined ? sourceNumerator!
    : BigInt(integer(settings.frameRate, "Frame rate", 1, 120));
  const denominator = settings.frameRate === undefined ? sourceDenominator! : 1n;
  const durationS = info.sampleCount / info.sampleRate;
  const countDivisor = BigInt(info.sampleRate) * denominator;
  const frameCount = settings.frameRate === undefined ? info.frameCount
    : Number((BigInt(info.sampleCount) * numerator + countDivisor - 1n) / countDivisor);
  integer(frameCount, "Export frame count", 1, Number.MAX_SAFE_INTEGER);
  const timestamp = (frame: number) => Number((BigInt(frame) * denominator * 1_000_000n + numerator / 2n) / numerator) / 1_000_000;
  return {
    width, height, frameCount, audioBitrate, durationS,
    frameRate: `${numerator}/${denominator}`,
    fps: Number(numerator) / Number(denominator),
    partialFinalFrame: settings.frameRate !== undefined && BigInt(info.sampleCount) * numerator % countDivisor !== 0n,
    rasterScale: Math.min(width / info.canvasWidth, height / info.canvasHeight),
    resize: width !== info.canvasWidth || height !== info.canvasHeight,
    sourceFrame(frame: number) {
      return Math.min(info.frameCount - 1, Number(BigInt(frame) * denominator * sourceNumerator! / (numerator * sourceDenominator!)));
    },
    timestamp,
    endTimestamp(frame: number) {
      const next = timestamp(frame + 1);
      return settings.frameRate === undefined ? next : Math.min(next, Math.round(durationS * 1_000_000) / 1_000_000);
    },
  };
}
