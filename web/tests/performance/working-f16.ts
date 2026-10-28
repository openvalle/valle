/** Pre-delivery linear Rec.2020, premultiplied RGBA binary16. Never clamp to SDR. */
export function halfValue(bits: number): number {
  const sign = bits & 0x8000 ? -1 : 1,
    exponent = (bits >>> 10) & 31,
    mantissa = bits & 1023;
  return exponent === 31
    ? mantissa
      ? NaN
      : sign * Infinity
    : sign * (exponent === 0 ? mantissa * 2 ** -24 : (1 + mantissa / 1024) * 2 ** (exponent - 15));
}
export function inspectWorkingF16(bytes: Uint8Array) {
  if (!bytes.length || bytes.length % 8) throw new Error("invalid working F16 storage");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let minimum = Infinity,
    maximum = -Infinity,
    nonfinite = 0,
    negative = 0,
    hdr = 0;
  for (let i = 0; i < bytes.length; i += 2) {
    const value = halfValue(view.getUint16(i, true));
    if (!Number.isFinite(value)) {
      nonfinite++;
      continue;
    }
    minimum = Math.min(minimum, value);
    maximum = Math.max(maximum, value);
    if ((i / 2) % 4 !== 3) {
      negative += Number(value < 0);
      hdr += Number(value > 1);
    }
  }
  return { minimum, maximum, nonfinite, negative, hdr };
}
export function compareWorkingF16(
  reference: Uint8Array,
  actual: Uint8Array,
  rule: { rgb: number; alpha: number },
) {
  if (
    !reference.length ||
    reference.length !== actual.length ||
    reference.length % 8 ||
    ![rule.rgb, rule.alpha].every((v) => Number.isFinite(v) && v >= 0)
  )
    throw new Error("invalid working F16 comparison");
  const a = new DataView(reference.buffer, reference.byteOffset, reference.byteLength);
  const b = new DataView(actual.buffer, actual.byteOffset, actual.byteLength);
  let rgbMax = 0,
    alphaMax = 0,
    whiteMax = 0,
    failedPixels = 0,
    nonfinite = 0,
    differentBits = 0;
  for (let i = 0; i < reference.length; i += 8) {
    const aa = halfValue(a.getUint16(i + 6, true)),
      ab = halfValue(b.getUint16(i + 6, true));
    const alpha = Math.abs(aa - ab);
    alphaMax = Math.max(alphaMax, alpha);
    let fail = alpha > rule.alpha;
    for (let c = 0; c < 4; c++) {
      const av = halfValue(a.getUint16(i + c * 2, true)),
        bv = halfValue(b.getUint16(i + c * 2, true));
      differentBits += Number(a.getUint16(i + c * 2, true) !== b.getUint16(i + c * 2, true));
      if (
        !Number.isFinite(av) ||
        !Number.isFinite(bv) ||
        !Number.isFinite(aa) ||
        !Number.isFinite(ab)
      ) {
        nonfinite++;
        fail = true;
        continue;
      }
      if (c === 3) continue;
      const rgb = Math.abs(av - bv),
        white = Math.abs(av - aa - (bv - ab));
      rgbMax = Math.max(rgbMax, rgb);
      whiteMax = Math.max(whiteMax, white);
      if (rgb > rule.rgb || white > rule.rgb) fail = true;
    }
    failedPixels += Number(fail);
  }
  return {
    pass: failedPixels === 0,
    rgbMax,
    alphaMax,
    whiteMax,
    failedPixels,
    nonfinite,
    differentBits,
  };
}
