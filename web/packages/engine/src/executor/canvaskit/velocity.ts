import type { DeviceRoi } from "./executor.ts";

/** The one-draw GPU ABI; null retains Skia's filter tree where its coverage differs. */
export function velocityBlurUniforms(
  input: DeviceRoi,
  output: DeviceRoi,
  from: readonly number[],
  to: readonly number[],
): Float32Array | null {
  const start = from.map(Math.fround),
    end = to.map(Math.fround);
  const reach = Math.max(...start.map(Math.abs), ...end.map(Math.abs));
  const largest = Math.max(2048, 2 * Math.max(output.width, output.height));
  if (
    reach <= 1e-6 ||
    input.width > largest ||
    input.height > largest ||
    Math.abs(input.x) >= 1_048_576 ||
    Math.abs(input.y) >= 1_048_576
  )
    return null;
  const samples = Math.min(32, Math.max(9, Math.ceil(reach) + 1));
  const coordinate = Math.max(
    Math.abs(input.x) + input.width,
    Math.abs(input.y) + input.height,
    Math.abs(output.x) + output.width,
    Math.abs(output.y) + output.height,
  );
  const tieMargin = Math.max(0.001, coordinate * 2 ** -21);
  const uniforms = new Float32Array(90);
  uniforms.set([
    input.width,
    input.height,
    input.x,
    input.y,
    samples,
    1,
    1,
    0,
    0,
    0,
    0,
    1,
    0,
    0,
    0,
    0,
    1,
    0,
    0,
    0,
    0,
    1 / samples,
    0,
    0,
    0,
    0,
  ]);
  for (let i = 0; i < samples; i++) {
    const phase = Math.fround(Math.fround((2 * i) / (samples - 1)) - 1);
    const point = phase < 0 ? start : end;
    for (let axis = 0; axis < 2; axis++) {
      const offset = Math.fround(point[axis]! * Math.abs(phase));
      // GPU normalized texture coordinates make a half-texel tie sensitive to the original
      // filter's intermediate placements. Preserve that draw, including near ties.
      if (Math.abs(offset - Math.floor(offset) - 0.5) <= tieMargin) return null;
      uniforms[26 + i * 2 + axis] = offset;
    }
  }
  return uniforms;
}
