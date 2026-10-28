import type { RgbaImage } from "../parity/lib/metrics.ts";

/** Registered per fixture. Range is measured on the error (actual - reference), not on
 * changing scene pixels. A range of 2 accepts normal -1/+1 quantization variation. */
export interface TemporalRule {
  windowFrames: number;
  rgbRange: number;
  alphaRange: number;
  minimumPixels: number;
  minimumWindowRun: number;
}
export interface MotionTrack {
  name: string;
  width: number;
  height: number;
  positions: { frame: number; x: number; y: number }[];
}
export interface TemporalFrame {
  frame: number;
  reference: RgbaImage;
  actual: RgbaImage;
}

/** Full-frame background plus explicitly tracked object regions. Screen coordinates covered
 * by a moving object reset their history; object-local coordinates carry that history instead.
 * No optical-flow guesses, spatial averages, edge allowances, or sign-only failure rules. */
export function compareSequence(
  frames: TemporalFrame[],
  rule: TemporalRule,
  tracks: MotionTrack[] = [],
) {
  const first = frames[0]?.reference;
  if (
    !first ||
    !Number.isSafeInteger(rule.windowFrames) ||
    rule.windowFrames < 2 ||
    rule.windowFrames > 128 ||
    frames.length < rule.windowFrames ||
    ![rule.rgbRange, rule.alphaRange].every((v) => Number.isFinite(v) && v >= 0) ||
    !Number.isSafeInteger(rule.minimumPixels) ||
    rule.minimumPixels < 1 ||
    !Number.isSafeInteger(rule.minimumWindowRun) ||
    rule.minimumWindowRun < 1 ||
    rule.minimumWindowRun > frames.length - rule.windowFrames + 1
  )
    throw new Error("invalid temporal comparison or insufficient continuous frames");
  const { width, height } = first;
  if (![width, height].every((v) => Number.isSafeInteger(v) && v > 0))
    throw new Error("invalid temporal dimensions");
  for (const [i, row] of frames.entries()) {
    if (
      !Number.isSafeInteger(row.frame) ||
      row.frame < 0 ||
      (i && row.frame !== frames[i - 1]!.frame + 1)
    )
      throw new Error("temporal comparison requires consecutive, unique frame numbers");
    for (const image of [row.reference, row.actual])
      if (
        image.width !== width ||
        image.height !== height ||
        image.data.length !== width * height * 4
      )
        throw new Error("temporal frame dimensions differ");
  }
  const names = new Set<string>(["background"]);
  for (const track of tracks) {
    if (
      !track.name ||
      names.has(track.name) ||
      track.positions.length !== frames.length ||
      ![track.width, track.height].every((v) => Number.isSafeInteger(v) && v > 0)
    )
      throw new Error("invalid temporal track");
    names.add(track.name);
    for (const [i, position] of track.positions.entries())
      if (
        position.frame !== frames[i]!.frame ||
        ![position.x, position.y].every(Number.isSafeInteger) ||
        position.x < 0 ||
        position.y < 0 ||
        position.x + track.width > width ||
        position.y + track.height > height
      )
        throw new Error("temporal track leaves the frame or has a missing position");
  }

  const regions = [null, ...tracks].map((track) => {
    const rw = track?.width ?? width,
      rh = track?.height ?? height;
    let rgbRangeMax = 0,
      alphaRangeMax = 0,
      whiteRangeMax = 0;
    let affectedPixels = 0,
      maximumWindowRun = 0,
      signChanges = 0,
      comparedWindows = 0;
    const affectedAtFrame = new Uint32Array(frames.length);
    // O(frames) scratch, reused for each pixel. Work scales with captured pixels and a
    // bounded window; no retained float image for every captured frame is necessary.
    const errors = new Float64Array(frames.length * 7);
    const valid = new Uint8Array(frames.length);
    for (let y = 0; y < rh; y++)
      for (let x = 0; x < rw; x++) {
        for (let t = 0; t < frames.length; t++) {
          const position = track?.positions[t];
          const sx = x + (position?.x ?? 0),
            sy = y + (position?.y ?? 0);
          const covered =
            !track &&
            tracks.some((other) => {
              const p = other.positions[t]!;
              return sx >= p.x && sy >= p.y && sx < p.x + other.width && sy < p.y + other.height;
            });
          valid[t] = covered ? 0 : 1;
          if (covered) continue;
          const row = frames[t]!,
            pixel = (sy * width + sx) * 4,
            offset = t * 7;
          const a = row.reference.data[pixel + 3]!,
            b = row.actual.data[pixel + 3]!;
          errors[offset + 3] = b - a;
          for (let c = 0; c < 3; c++) {
            const error =
              (row.actual.data[pixel + c]! * b - row.reference.data[pixel + c]! * a) / 255;
            errors[offset + c] = error;
            errors[offset + 4 + c] = error - (b - a);
          }
        }
        let history = 0,
          run = 0,
          pixelRun = 0;
        for (let t = 0; t < frames.length; t++) {
          if (!valid[t]) {
            history = 0;
            run = 0;
            continue;
          }
          history++;
          if (history > 1)
            for (let c = 0; c < 7; c++)
              if (errors[t * 7 + c]! * errors[(t - 1) * 7 + c]! < 0) signChanges++;
          if (history < rule.windowFrames) continue;
          comparedWindows++;
          let failed = false;
          for (let c = 0; c < 7; c++) {
            let low = Infinity,
              high = -Infinity;
            for (let j = t - rule.windowFrames + 1; j <= t; j++) {
              const value = errors[j * 7 + c]!;
              low = Math.min(low, value);
              high = Math.max(high, value);
            }
            const range = high - low;
            if (c < 3) rgbRangeMax = Math.max(rgbRangeMax, range);
            else if (c === 3) alphaRangeMax = Math.max(alphaRangeMax, range);
            else whiteRangeMax = Math.max(whiteRangeMax, range);
            if (range > (c === 3 ? rule.alphaRange : rule.rgbRange)) failed = true;
          }
          run = failed ? run + 1 : 0;
          pixelRun = Math.max(pixelRun, run);
          if (failed) affectedAtFrame[t]++;
        }
        maximumWindowRun = Math.max(maximumWindowRun, pixelRun);
        if (pixelRun >= rule.minimumWindowRun) affectedPixels++;
      }
    return {
      region: track?.name ?? "background",
      pass: affectedPixels < rule.minimumPixels,
      rgbRangeMax,
      alphaRangeMax,
      whiteRangeMax,
      affectedPixels,
      maximumWindowRun,
      maximumAffectedSpanFrames: maximumWindowRun ? maximumWindowRun + rule.windowFrames - 1 : 0,
      comparedWindows,
      signChanges,
      affectedAtFrame: frames.map((row, i) => ({ frame: row.frame, pixels: affectedAtFrame[i]! })),
    };
  });
  if (!regions.some((r) => r.comparedWindows > 0))
    throw new Error("no comparable temporal regions");
  return { pass: regions.every((r) => r.pass), frames: frames.length, rule, regions };
}
