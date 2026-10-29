import { expect, test } from "bun:test";
import { resolve } from "node:path";
import type { CanvasKit } from "canvaskit-wasm";
import { fontFaceBytes } from "./font-face.ts";

const font = async (weight: string) =>
  new Uint8Array(
    await Bun.file(
      resolve(import.meta.dir, `../../../../../../assets/fonts/katex/KaTeX_Main-${weight}.ttf`),
    ).arrayBuffer(),
  );

function collection(fonts: Uint8Array[]): Uint8Array {
  const size =
    12 + fonts.length * 4 + fonts.reduce((sum, bytes) => sum + Math.ceil(bytes.length / 4) * 4, 0);
  const result = new Uint8Array(size),
    view = new DataView(result.buffer);
  view.setUint32(0, 0x74746366);
  view.setUint32(4, 0x10000);
  view.setUint32(8, fonts.length);
  let start = 12 + fonts.length * 4;
  fonts.forEach((bytes, index) => {
    view.setUint32(12 + index * 4, start);
    result.set(bytes, start);
    const tables = view.getUint16(start + 4);
    for (let table = 0; table < tables; table++) {
      const at = start + 12 + table * 16 + 8;
      view.setUint32(at, view.getUint32(at) + start);
    }
    start += Math.ceil(bytes.length / 4) * 4;
  });
  return result;
}

test("CanvasKit reads the exact selected collection face", async () => {
  const { default: init } = (await import("canvaskit-wasm")) as unknown as {
    default(options: { locateFile(file: string): string }): Promise<CanvasKit>;
  };
  const ck = await init({
    locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm", import.meta.dir),
  });
  const fonts = await Promise.all([font("Regular"), font("Bold")]);
  const ttc = collection(fonts);
  const widths: number[][] = [];
  for (let index = 0; index < fonts.length; index++) {
    const face = ck.Typeface.MakeFreeTypeFaceFromData(fontFaceBytes(ttc, index).buffer)!;
    const reference = ck.Typeface.MakeFreeTypeFaceFromData(fonts[index]!.slice().buffer)!;
    expect(face).not.toBeNull();
    const actualFont = new ck.Font(face, 40),
      referenceFont = new ck.Font(reference, 40);
    try {
      const glyphs = face.getGlyphIDs("TTC bold");
      expect([...glyphs]).toEqual([...reference.getGlyphIDs("TTC bold")]);
      const actual = [...actualFont.getGlyphWidths(glyphs)];
      expect(actual).toEqual([...referenceFont.getGlyphWidths(glyphs)]);
      widths.push(actual);
    } finally {
      actualFont.delete();
      referenceFont.delete();
      face.delete();
      reference.delete();
    }
  }
  expect(widths[0]).not.toEqual(widths[1]);
  // A typed-array view need not start at byte zero in its backing buffer.
  const padded = new Uint8Array(ttc.length + 9);
  padded.set(ttc, 9);
  expect(fontFaceBytes(padded.subarray(9), 1)).toEqual(fontFaceBytes(ttc, 1));
  expect(() => fontFaceBytes(ttc, 2)).toThrow();
  expect(() => fontFaceBytes(fonts[0]!, 1)).toThrow();
  expect(() => fontFaceBytes(ttc.subarray(0, 16), 0)).toThrow();
  const corrupt = ttc.slice();
  new DataView(corrupt.buffer).setUint32(12, 0xffffffff);
  expect(() => fontFaceBytes(corrupt, 0)).toThrow();
});
