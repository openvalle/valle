import { describe, expect, test } from "bun:test";

import { sha256 } from "../../abi/sha256.ts";
import {
  BATCH_INSTANCE_BYTES,
  DRAW_PROGRAM_ABI,
  PackedDrawProgramError,
  applyDrawProgramPatch,
  decodeDrawProgram,
} from "./draw-program.ts";
import { PROGRAM_PATCH_ABI } from "../../generated/packed-abi.ts";

const HEADER_BYTES = DRAW_PROGRAM_ABI.headerBytes;
const ENTRY_BYTES = DRAW_PROGRAM_ABI.sectionEntryBytes;
const KINDS = [1, 2, 7, 3, 4, 5, 6] as const;

function json(value: unknown): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(value));
}

function fixture(): Uint8Array {
  const batch = new Uint8Array(BATCH_INSTANCE_BYTES);
  const batchView = new DataView(batch.buffer);
  batchView.setFloat64(0, 4, true);
  batchView.setFloat64(8, 5, true);
  batchView.setFloat64(16, 6, true);
  batchView.setFloat64(24, 7, true);
  batchView.setFloat32(32, 0.25, true);
  batchView.setFloat32(36, 0.5, true);
  batchView.setFloat32(40, 0.75, true);
  batchView.setFloat32(44, 1, true);
  const sections = [
    json([0]),
    json([{ kind: "geometryBatch", value: { geometry: "rect", instances: { start: 0, count: 1 } } }]),
    batch,
    json([]),
    json([]),
    json({ x: 0, y: 0, width: 16, height: 9 }),
    json({}),
  ];
  const tableBytes = HEADER_BYTES + ENTRY_BYTES * sections.length;
  const total = tableBytes + sections.reduce((sum, section) => sum + section.byteLength, 0);
  const bytes = new Uint8Array(total);
  const view = new DataView(bytes.buffer);
  bytes.set(new TextEncoder().encode(DRAW_PROGRAM_ABI.magic), 0);
  view.setUint32(8, DRAW_PROGRAM_ABI.formatVersion, true);
  view.setUint32(12, DRAW_PROGRAM_ABI.endiannessMarker, true);
  view.setUint16(16, sections.length, true);
  view.setBigUint64(20, BigInt(total), true);
  let offset = tableBytes;
  for (const [index, section] of sections.entries()) {
    const entry = HEADER_BYTES + index * ENTRY_BYTES;
    view.setUint16(entry, KINDS[index]!, true);
    view.setUint32(entry + 4, index === 2 ? 1 : index === 5 || index === 6 ? 1 : JSON.parse(new TextDecoder().decode(section)).length, true);
    view.setBigUint64(entry + 8, BigInt(offset), true);
    view.setBigUint64(entry + 16, BigInt(section.byteLength), true);
    bytes.set(section, offset);
    offset += section.byteLength;
  }
  seal(bytes);
  return bytes;
}

function seal(bytes: Uint8Array): void {
  const end = DRAW_PROGRAM_ABI.checksumOffset + DRAW_PROGRAM_ABI.checksumBytes;
  bytes.fill(0, DRAW_PROGRAM_ABI.checksumOffset, end);
  bytes.set(sha256(bytes), DRAW_PROGRAM_ABI.checksumOffset);
}

function findBytes(haystack: Uint8Array, needle: Uint8Array): number {
  outer: for (let offset = 0; offset <= haystack.byteLength - needle.byteLength; offset += 1) {
    for (let index = 0; index < needle.byteLength; index += 1) {
      if (haystack[offset + index] !== needle[index]) continue outer;
    }
    return offset;
  }
  return -1;
}

function identityPatch(target: Uint8Array): Uint8Array {
  const patch = new Uint8Array(PROGRAM_PATCH_ABI.headerBytes + 9);
  const view = new DataView(patch.buffer);
  patch.set(new TextEncoder().encode(PROGRAM_PATCH_ABI.magic));
  view.setUint32(8, target.byteLength, true);
  view.setUint32(12, 1, true);
  patch[16] = 0;
  view.setUint32(17, 0, true);
  view.setUint32(21, target.byteLength, true);
  return patch;
}

describe("DrawProgram packed GeometryBatch", () => {
  test("admits the fixed-width table and reconstructs a random-access identity patch", async () => {
    const bytes = fixture();
    const draw = await decodeDrawProgram(bytes);
    expect(draw.batchInstances.count).toBe(1);
    expect(draw.batchInstances.view.getFloat64(16, true)).toBe(6);
    expect(applyDrawProgramPatch(bytes, identityPatch(bytes))).toEqual(bytes);
  });

  test("rejects malformed instance values and non-canonical ranges after checksum admission", async () => {
    const invalidSize = fixture();
    const view = new DataView(invalidSize.buffer);
    const batchOffset = Number(view.getBigUint64(60 + 2 * ENTRY_BYTES + 8, true));
    view.setFloat64(batchOffset + 16, 0, true);
    seal(invalidSize);
    await expect(decodeDrawProgram(invalidSize)).rejects.toBeInstanceOf(PackedDrawProgramError);

    const invalidRange = fixture();
    const rangeOffset = findBytes(invalidRange, new TextEncoder().encode('"start":0'));
    expect(rangeOffset).toBeGreaterThan(0);
    invalidRange[rangeOffset + 8] = "1".charCodeAt(0);
    seal(invalidRange);
    await expect(decodeDrawProgram(invalidRange)).rejects.toThrow("canonically partition");
  });

  test("rejects the wrong exact format version even with a valid checksum", async () => {
    const bytes = fixture();
    new DataView(bytes.buffer).setUint32(8, DRAW_PROGRAM_ABI.formatVersion + 1, true);
    seal(bytes);

    await expect(decodeDrawProgram(bytes)).rejects.toThrow(
      `unsupported format version ${DRAW_PROGRAM_ABI.formatVersion + 1}`,
    );
  });

  test("rejects impossible section lengths and escaping patch copies", async () => {
    const invalidSection = fixture();
    const view = new DataView(invalidSection.buffer);
    view.setBigUint64(60 + 2 * ENTRY_BYTES + 16, BigInt(BATCH_INSTANCE_BYTES - 1), true);
    seal(invalidSection);
    await expect(decodeDrawProgram(invalidSection)).rejects.toThrow("47 bytes");

    const bytes = fixture();
    const patch = identityPatch(bytes);
    new DataView(patch.buffer).setUint32(21, bytes.byteLength + 1, true);
    expect(() => applyDrawProgramPatch(bytes, patch)).toThrow("escapes the baseline");
  });

  test("rejects the removed versioned ProgramPatch header", () => {
    const bytes = fixture();
    const unsupportedPatch = new Uint8Array(29);
    const view = new DataView(unsupportedPatch.buffer);
    unsupportedPatch.set(new TextEncoder().encode(PROGRAM_PATCH_ABI.magic));
    view.setUint16(8, 1, true);
    view.setUint32(12, bytes.byteLength, true);
    view.setUint32(16, 1, true);
    unsupportedPatch[20] = 0;
    view.setUint32(25, bytes.byteLength, true);

    expect(() => applyDrawProgramPatch(bytes, unsupportedPatch)).toThrow(
      "program patch instruction count is impossible",
    );
  });
});
