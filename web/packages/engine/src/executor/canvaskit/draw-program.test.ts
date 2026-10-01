import { describe, expect, test } from "bun:test";

import {
  BATCH_INSTANCE_BYTES,
  DRAW_PROGRAM_ABI,
  applyDrawProgramPatch,
  decodeDrawProgram,
} from "./draw-program.ts";
import { PROGRAM_PATCH_ABI } from "../../generated/packed-abi.ts";

const HEADER_BYTES = DRAW_PROGRAM_ABI.headerBytes;
const ENTRY_BYTES = DRAW_PROGRAM_ABI.sectionEntryBytes;
const KINDS = [1, 2, 7, 8, 9, 3, 4, 5, 6] as const;

function json(value: unknown): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(value));
}

function fixture(
  geometry: "rect" | "path" | "roundRect" | "image" = "rect",
  path: number | null = null,
  strokeColor: [number, number, number, number] | null = null,
  pathStyle: { fill: boolean; dash: number[]; dashOffset: number; cap: string; join: string; miterLimit: number } | null = null,
  dashOffset: number | null = null,
  atlasSrc = { x: 0, y: 0, width: 0.5, height: 1 },
  atlasTexture: Record<string, unknown> = { key: "asset://sprites", kind: "image", colorDomain: "linearRec2020", alpha: "premultiplied", sampleTimeMicros: null },
): Uint8Array {
  const batch = new Uint8Array(BATCH_INSTANCE_BYTES);
  const batchView = new DataView(batch.buffer);
  for (const [index, value] of [6, 0, 0, 7, 4, 5].entries()) batchView.setFloat64(index * 8, value, true);
  batchView.setFloat32(48, 0.25, true);
  batchView.setFloat32(52, 0.5, true);
  batchView.setFloat32(56, 0.75, true);
  batchView.setFloat32(60, 1, true);
  batchView.setFloat32(64, 1, true);
  batchView.setFloat32(68, 0, true);
  const shape = geometry === "image"
    ? { kind: geometry, value: { texture: atlasTexture, src: atlasSrc, sampling: "linearClamp" } }
    : geometry === "roundRect"
    ? { kind: geometry, value: { rect: { x: 0, y: 0, width: 1, height: 1 }, radii: [[0.2, 0.2], [0.2, 0.2], [0.2, 0.2], [0.2, 0.2]] } }
    : geometry === "path" || path !== null ? { kind: geometry, value: path } : { kind: geometry };
  const strokeBytes = new Uint8Array(strokeColor === null ? 0 : 16);
  if (strokeColor !== null) {
    const strokeView = new DataView(strokeBytes.buffer);
    for (const [index, value] of strokeColor.entries()) strokeView.setFloat32(index * 4, value, true);
  }
  const dashBytes = new Uint8Array(dashOffset === null ? 0 : 4);
  if (dashOffset !== null) new DataView(dashBytes.buffer).setFloat32(0, dashOffset, true);
  const sections = [
    json([0]),
    json([{ kind: "instanceBatch", value: { shape, instances: { start: 0, count: 1 }, strokeColors: strokeColor === null ? null : { start: 0, count: 1 }, dashOffsets: dashOffset === null ? null : { start: 0, count: 1 }, pathStyle } }]),
    batch,
    strokeBytes,
    dashBytes,
    json(geometry === "path" ? [{ verbs: ["moveTo", "lineTo", "lineTo", "close"], points: [[0, 0], [5, 0], [0, 5]] }] : []),
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
    view.setUint32(entry + 4, index === 2 ? 1 : index === 3 ? strokeBytes.byteLength / 16 : index === 4 ? dashBytes.byteLength / 4 : index === 7 || index === 8 ? 1 : JSON.parse(new TextDecoder().decode(section)).length, true);
    view.setBigUint64(entry + 8, BigInt(offset), true);
    view.setBigUint64(entry + 16, BigInt(section.byteLength), true);
    bytes.set(section, offset);
    offset += section.byteLength;
  }
  return bytes;
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

describe("DrawProgram packed InstanceBatch", () => {
  test("admits atlas regions and rejects invalid crops and strokes", async () => {
    const draw = await decodeDrawProgram(fixture("image"));
    expect(draw.nodes[0]?.kind).toBe("instanceBatch");
    await expect(decodeDrawProgram(fixture("image", null, null, null, null,
      { x: 0.8, y: 0, width: 0.5, height: 1 }))).rejects.toThrow("shape.value is invalid");
    await expect(decodeDrawProgram(fixture("image", null, null, null, null, undefined,
      { key: "asset://sprites", kind: "image", colorDomain: "data", alpha: "premultiplied", sampleTimeMicros: null }))).rejects.toThrow("shape.value is invalid");
    const stroke = fixture("image");
    const view = new DataView(stroke.buffer);
    const offset = Number(view.getBigUint64(HEADER_BYTES + 2 * ENTRY_BYTES + 8, true));
    view.setFloat32(offset + 68, 1, true);
    await expect(decodeDrawProgram(stroke)).rejects.toThrow("has a stroke");
  });
  test("admits a shared path and rejects mismatched or out-of-range path references", async () => {
    const draw = await decodeDrawProgram(fixture("path", 0));
    expect(draw.paths).toHaveLength(1);
    await expect(decodeDrawProgram(fixture("path", null))).rejects.toThrow("shape.value");
    await expect(decodeDrawProgram(fixture("path", 1))).rejects.toThrow("out of range");
    await expect(decodeDrawProgram(fixture("rect", 0))).rejects.toThrow("fields are not canonical");
    expect((await decodeDrawProgram(fixture("roundRect"))).nodes[0]?.kind).toBe("instanceBatch");
  });
  test("admits the fixed-width table and reconstructs a random-access identity patch", async () => {
    const bytes = fixture();
    const draw = await decodeDrawProgram(bytes);
    expect(draw.batchInstances.count).toBe(1);
    expect(draw.batchInstances.view.getFloat64(0, true)).toBe(6);
    expect(draw.batchInstances.view.getFloat64(32, true)).toBe(4);
    expect(draw.batchInstances.view.getFloat32(64, true)).toBe(1);
    expect(applyDrawProgramPatch(bytes, identityPatch(bytes))).toEqual(bytes);
  });

  test("admits an independent stroke color column and rejects malformed color values", async () => {
    const draw = await decodeDrawProgram(fixture("path", 0, [0.8, 0, 0, 1]));
    expect(draw.strokeColors.count).toBe(1);
    expect(draw.strokeColors.view.getFloat32(0, true)).toBeCloseTo(0.8);
    const invalid = fixture("path", 0, [0.8, 0, 0, 1]);
    const view = new DataView(invalid.buffer);
    const colorOffset = Number(view.getBigUint64(HEADER_BYTES + 3 * ENTRY_BYTES + 8, true));
    view.setFloat32(colorOffset + 12, Number.NaN, true);
    await expect(decodeDrawProgram(invalid)).rejects.toThrow("stroke color 0 is invalid");
  });

  test("admits a shared Path style and row dash phase, rejecting invalid phases", async () => {
    const style = { fill: false, dash: [3, 2], dashOffset: 0, cap: "round", join: "bevel", miterLimit: 6 };
    const draw = await decodeDrawProgram(fixture("path", 0, null, style, 1.25));
    expect(draw.dashOffsets.count).toBe(1);
    expect(draw.dashOffsets.view.getFloat32(0, true)).toBe(1.25);
    await expect(decodeDrawProgram(fixture("path", 0, null, style, Number.NaN))).rejects.toThrow("dash offset 0 is non-finite");
    await expect(decodeDrawProgram(fixture("path", 0, null, { ...style, dash: [0, 0] }, 1.25))).rejects.toThrow("pathStyle is invalid");
  });

  test("rejects malformed instance values and non-canonical ranges", async () => {
    const singular = fixture();
    const view = new DataView(singular.buffer);
    const batchOffset = Number(view.getBigUint64(HEADER_BYTES + 2 * ENTRY_BYTES + 8, true));
    view.setFloat64(batchOffset, 0, true);
    await expect(decodeDrawProgram(singular)).rejects.toThrow("singular transform");

    const invalidRotation = fixture();
    const rotationView = new DataView(invalidRotation.buffer);
    const rotationOffset = Number(rotationView.getBigUint64(HEADER_BYTES + 2 * ENTRY_BYTES + 8, true));
    rotationView.setFloat64(rotationOffset + 8, Number.NaN, true);
    await expect(decodeDrawProgram(invalidRotation)).rejects.toThrow("non-finite");

    const invalidOpacity = fixture();
    const opacityView = new DataView(invalidOpacity.buffer);
    const opacityOffset = Number(opacityView.getBigUint64(HEADER_BYTES + 2 * ENTRY_BYTES + 8, true));
    opacityView.setFloat32(opacityOffset + 64, 1.1, true);
    await expect(decodeDrawProgram(invalidOpacity)).rejects.toThrow("outside [0,1]");

    const invalidRange = fixture();
    const rangeOffset = findBytes(invalidRange, new TextEncoder().encode('"start":0'));
    expect(rangeOffset).toBeGreaterThan(0);
    invalidRange[rangeOffset + 8] = "1".charCodeAt(0);
    await expect(decodeDrawProgram(invalidRange)).rejects.toThrow("canonically partition");
  });

  test("rejects the wrong exact format version", async () => {
    const bytes = fixture();
    new DataView(bytes.buffer).setUint32(8, DRAW_PROGRAM_ABI.formatVersion + 1, true);

    await expect(decodeDrawProgram(bytes)).rejects.toThrow(
      `unsupported format version ${DRAW_PROGRAM_ABI.formatVersion + 1}`,
    );
  });

  test("rejects impossible section lengths and escaping patch copies", async () => {
    const invalidSection = fixture();
    const view = new DataView(invalidSection.buffer);
    view.setBigUint64(HEADER_BYTES + 2 * ENTRY_BYTES + 16, BigInt(BATCH_INSTANCE_BYTES - 1), true);
    await expect(decodeDrawProgram(invalidSection)).rejects.toThrow(`${BATCH_INSTANCE_BYTES - 1} bytes`);

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
