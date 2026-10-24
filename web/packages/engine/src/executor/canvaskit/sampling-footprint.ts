export interface SampleRegion {
  x: number;
  y: number;
  width: number;
  height: number;
}
export interface SampleFilter {
  kind: string;
  offset?: unknown;
  k1?: number;
  k2?: number;
  [key: string]: unknown;
}

// Conservative device-space sample bounds. These never change the filter's original frame.
export const pointFilters = new Set([
  "colorMatrix",
  "brightness",
  "contrast",
  "grayscale",
  "hueRotate",
  "invert",
  "opacity",
  "saturate",
  "sepia",
  "filmGrain",
]);

export function samplingFootprint(
  filter: SampleFilter,
  output: SampleRegion,
  frame?: readonly number[],
): SampleRegion | null {
  if (pointFilters.has(filter.kind)) return { ...output };
  if (filter.kind === "chromaticAberration") {
    if (
      !Array.isArray(filter.offset) ||
      filter.offset.length !== 2 ||
      !filter.offset.every(Number.isFinite)
    )
      return null;
    const dx = Math.abs(filter.offset[0]) / 2 + 2,
      dy = Math.abs(filter.offset[1]) / 2 + 2;
    const x = Math.floor(output.x - dx),
      y = Math.floor(output.y - dy);
    return {
      x,
      y,
      width: Math.ceil(output.x + output.width + dx) - x,
      height: Math.ceil(output.y + output.height + dy) - y,
    };
  }
  if (
    filter.kind !== "lensDistortion" ||
    !frame ||
    frame.length !== 4 ||
    !frame.every(Number.isFinite)
  )
    return null;
  const [fx, fy, w, h] = frame.map(Math.fround);
  const k1 = Math.fround(filter.k1 ?? NaN),
    k2 = Math.fround(filter.k2 ?? NaN);
  if (!Number.isFinite(k1) || !Number.isFinite(k2) || w <= 0 || h <= 0) return null;
  // The zero-distortion shader bypasses the frame test and performs an integer lookup.
  if (k1 === 0 && k2 === 0) return { ...output };
  const x = Math.max(output.x, fx),
    y = Math.max(output.y, fy);
  const right = Math.min(output.x + output.width, fx + w),
    bottom = Math.min(output.y + output.height, fy + h);
  if (right <= x || bottom <= y) return null;
  const cx = fx + w / 2,
    cy = fy + h / 2,
    dx = [x - cx, right - cx],
    dy = [y - cy, bottom - cy];
  const squaredMin = (range: number[]) =>
    range[0] <= 0 && range[1] >= 0 ? 0 : Math.min(range[0] ** 2, range[1] ** 2);
  const squaredMax = (range: number[]) => Math.max(range[0] ** 2, range[1] ** 2);
  const radius = (w * w + h * h) / 4;
  const minimum = Math.min((squaredMin(dx) + squaredMin(dy)) / radius, 1);
  const maximum = Math.min((squaredMax(dx) + squaredMax(dy)) / radius, 1);
  const scale = (r: number) => 1 + k1 * r + k2 * r * r,
    scales = [scale(minimum), scale(maximum)];
  const extremum = -k1 / (2 * k2);
  if (Number.isFinite(extremum) && extremum > minimum && extremum < maximum)
    scales.push(scale(extremum));
  const s = [Math.min(...scales), Math.max(...scales)];
  const product = (range: number[]) => [
    range[0] * s[0],
    range[0] * s[1],
    range[1] * s[0],
    range[1] * s[1],
  ];
  const px = product(dx),
    py = product(dy);
  // Two pixels cover bilinear taps; grow the guard for large float32 device coordinates.
  const magnitude = Math.max(
    Math.abs(cx),
    Math.abs(cy),
    w,
    h,
    ...px.map(Math.abs),
    ...py.map(Math.abs),
  );
  const guard = 2 + Math.ceil(magnitude * 2 ** -20);
  const left = Math.floor(cx + Math.min(...px)) - guard,
    top = Math.floor(cy + Math.min(...py)) - guard;
  return {
    x: left,
    y: top,
    width: Math.ceil(cx + Math.max(...px)) + guard - left,
    height: Math.ceil(cy + Math.max(...py)) + guard - top,
  };
}
