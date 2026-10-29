/** CanvasKit decodes face zero only. Materialize the selected TTC/OTC face as an
 * ordinary SFNT without changing glyph tables, variations or the resource's identity. */
export function fontFaceBytes(bytes: Uint8Array, index: number): Uint8Array<ArrayBuffer> {
  const invalid = (): never => {
    throw new Error(`invalid font collection face ${index}`);
  };
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const fits = (at: number, length: number) =>
    at >= 0 && length >= 0 && at + length <= bytes.length;
  if (!Number.isSafeInteger(index) || index < 0 || !fits(0, 12)) return invalid();
  if (view.getUint32(0) !== 0x74746366) {
    if (index !== 0) return invalid();
    return bytes.slice();
  }
  const count = view.getUint32(8);
  if (count === 0 || count > 256 || index >= count || !fits(12, count * 4)) return invalid();
  const start = view.getUint32(12 + index * 4);
  if (!fits(start, 12)) return invalid();
  const tables = view.getUint16(start + 4);
  const header = 12 + tables * 16;
  if (tables === 0 || tables > 4096 || !fits(start, header)) return invalid();
  const records: { from: number; length: number; to: number }[] = [];
  let size = header;
  for (let table = 0; table < tables; table++) {
    const at = start + 12 + table * 16;
    const from = view.getUint32(at + 8),
      length = view.getUint32(at + 12);
    if (!fits(from, length)) return invalid();
    records.push({ from, length, to: size });
    size += Math.ceil(length / 4) * 4;
    if (size > 64 * 1024 * 1024) return invalid();
  }
  const result = new Uint8Array(size);
  result.set(bytes.subarray(start, start + header));
  const output = new DataView(result.buffer);
  let adjustment: number | undefined;
  records.forEach(({ from, length, to }, table) => {
    const at = 12 + table * 16;
    output.setUint32(at + 8, to);
    result.set(bytes.subarray(from, from + length), to);
    if (output.getUint32(at) === 0x68656164 && length >= 12) {
      adjustment = to + 8;
      output.setUint32(adjustment, 0);
    }
  });
  if (adjustment !== undefined) {
    let checksum = 0;
    for (let at = 0; at < size; at += 4) checksum = (checksum + output.getUint32(at)) >>> 0;
    output.setUint32(adjustment, (0xb1b0afba - checksum) >>> 0);
  }
  return result;
}
