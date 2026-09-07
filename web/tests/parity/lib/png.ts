import { readBytes } from "./files.ts";
import type { RgbaImage } from "./metrics.ts";

const PNG_SIGNATURE = Uint8Array.from([137, 80, 78, 71, 13, 10, 26, 10]);
const decoder = new TextDecoder("ascii");

export async function readPng(path: string): Promise<RgbaImage> {
  const bytes = await readBytes(path);
  if (!bytesEqual(bytes.subarray(0, 8), PNG_SIGNATURE)) throw new Error(`${path} is not a PNG`);
  let offset = 8;
  let width = 0;
  let height = 0;
  const idat: Uint8Array[] = [];
  while (offset < bytes.length) {
    const length = readU32Be(bytes, offset);
    const type = decoder.decode(bytes.subarray(offset + 4, offset + 8));
    const data = bytes.subarray(offset + 8, offset + 8 + length);
    offset += 12 + length;
    if (type === "IHDR") {
      width = readU32Be(data, 0);
      height = readU32Be(data, 4);
      const bitDepth = data[8];
      const colorType = data[9];
      const interlace = data[12];
      if (bitDepth !== 8 || colorType !== 6 || interlace !== 0) {
        throw new Error(`${path} must be 8-bit non-interlaced RGBA PNG`);
      }
    } else if (type === "IDAT") {
      idat.push(data);
    } else if (type === "IEND") {
      break;
    }
  }
  const packed = concatenate(idat);
  const inflated = new Uint8Array(
    await new Response(
      new Blob([packed.buffer as ArrayBuffer])
        .stream()
        .pipeThrough(new DecompressionStream("deflate")),
    ).arrayBuffer(),
  );
  const stride = width * 4;
  const data = new Uint8Array(width * height * 4);
  let source = 0;
  for (let y = 0; y < height; y += 1) {
    const filter = inflated[source++]!;
    const row = inflated.subarray(source, source + stride);
    source += stride;
    const outStart = y * stride;
    unfilterRow(filter, row, data, outStart, y > 0 ? outStart - stride : -1, 4);
  }
  return { width, height, data };
}

function unfilterRow(
  filter: number,
  row: Uint8Array,
  out: Uint8Array,
  outStart: number,
  previousStart: number,
  bytesPerPixel: number,
): void {
  for (let x = 0; x < row.length; x += 1) {
    const left = x >= bytesPerPixel ? out[outStart + x - bytesPerPixel]! : 0;
    const up = previousStart >= 0 ? out[previousStart + x]! : 0;
    const upLeft = previousStart >= 0 && x >= bytesPerPixel
      ? out[previousStart + x - bytesPerPixel]!
      : 0;
    let value = row[x]!;
    if (filter === 1) value += left;
    else if (filter === 2) value += up;
    else if (filter === 3) value += Math.floor((left + up) / 2);
    else if (filter === 4) value += paeth(left, up, upLeft);
    else if (filter !== 0) throw new Error(`unsupported PNG filter ${filter}`);
    out[outStart + x] = value & 0xff;
  }
}

function paeth(left: number, up: number, upLeft: number): number {
  const prediction = left + up - upLeft;
  const leftDistance = Math.abs(prediction - left);
  const upDistance = Math.abs(prediction - up);
  const diagonalDistance = Math.abs(prediction - upLeft);
  if (leftDistance <= upDistance && leftDistance <= diagonalDistance) return left;
  return upDistance <= diagonalDistance ? up : upLeft;
}

function concatenate(chunks: readonly Uint8Array[]): Uint8Array {
  const result = new Uint8Array(chunks.reduce((sum, chunk) => sum + chunk.length, 0));
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk, offset);
    offset += chunk.length;
  }
  return result;
}

function readU32Be(bytes: Uint8Array, offset: number): number {
  return new DataView(bytes.buffer, bytes.byteOffset + offset, 4).getUint32(0, false);
}

function bytesEqual(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}
