import { sha256 } from "../../abi/sha256.ts";
import {
  DRAW_PROGRAM_ABI,
  PROGRAM_PATCH_ABI,
} from "../../generated/packed-abi.ts";

export { DRAW_PROGRAM_ABI };

const HEADER_BYTES = DRAW_PROGRAM_ABI.headerBytes;
const SECTION_ENTRY_BYTES = DRAW_PROGRAM_ABI.sectionEntryBytes;
const SECTION_COUNT = DRAW_PROGRAM_ABI.sectionCount;
const TABLE_BYTES = HEADER_BYTES + SECTION_ENTRY_BYTES * SECTION_COUNT;
export const BATCH_INSTANCE_BYTES = DRAW_PROGRAM_ABI.batchInstanceBytes;
const DRAW_LITTLE_ENDIAN = DRAW_PROGRAM_ABI.endianness === "little";
const EXPECTED_KINDS = [1, 2, 7, 3, 4, 5, 6] as const;
const PATCH_HEADER_BYTES = PROGRAM_PATCH_ABI.headerBytes;
const PATCH_COPY = 0;
const PATCH_INSERT = 1;

export interface RectWire { x: number; y: number; width: number; height: number }
export interface LinearColorWire { red: number; green: number; blue: number; alpha: number }
export interface PathWire { verbs: string[]; points: Array<[number, number]> }
export interface TaggedWire { kind: string; value?: unknown; [key: string]: unknown }
export interface BatchInstanceTableWire {
  readonly bytes: Uint8Array;
  readonly view: DataView;
  readonly count: number;
}
export interface DrawProgramWire {
  roots: number[];
  nodes: TaggedWire[];
  batchInstances: BatchInstanceTableWire;
  paths: PathWire[];
  paints: TaggedWire[];
  viewport: RectWire;
  requirements: Record<string, unknown>;
}

export class PackedDrawProgramError extends Error {
  constructor(message: string) {
    super(`packed DrawProgram: ${message}`);
    this.name = "PackedDrawProgramError";
  }
}

/**
 * Reconstruct one exact frame program from the immutable baseline in its RenderPlan template.
 * Patches are random-access and independently bounded; no prior frame is observable here.
 */
export function applyDrawProgramPatch(
  baselineInput: ArrayBuffer | ArrayBufferView,
  patchInput: ArrayBuffer | ArrayBufferView,
): Uint8Array {
  const baseline = exactBytes(baselineInput);
  const patch = exactBytes(patchInput);
  if (
    baseline.byteLength > PROGRAM_PATCH_ABI.maximumBytes
    || patch.byteLength > PROGRAM_PATCH_ABI.maximumBytes
  ) {
    fail("program patch byte budget exceeded");
  }
  if (patch.byteLength < PATCH_HEADER_BYTES) fail("program patch is truncated");
  const magic = new TextEncoder().encode(PROGRAM_PATCH_ABI.magic);
  if (!equal(patch.subarray(0, 8), magic)) fail("program patch has the wrong magic");
  const view = dataView(patch);
  const littleEndian = PROGRAM_PATCH_ABI.endianness === "little";
  const targetLength = view.getUint32(8, littleEndian);
  const instructionCount = view.getUint32(12, littleEndian);
  if (targetLength > PROGRAM_PATCH_ABI.maximumBytes) {
    fail("program patch output exceeds the byte budget");
  }
  if (instructionCount > Math.floor((patch.byteLength - PATCH_HEADER_BYTES) / 5)) {
    fail("program patch instruction count is impossible");
  }

  const output = new Uint8Array(targetLength);
  let cursor = PATCH_HEADER_BYTES;
  let outputOffset = 0;
  const takeU32 = (label: string): number => {
    if (cursor + 4 > patch.byteLength) fail(`program patch ${label} is truncated`);
    const value = view.getUint32(cursor, littleEndian);
    cursor += 4;
    return value;
  };
  for (let index = 0; index < instructionCount; index += 1) {
    if (cursor >= patch.byteLength) fail("program patch instruction is truncated");
    const tag = patch[cursor]!;
    cursor += 1;
    if (tag === PATCH_COPY) {
      const offset = takeU32("copy offset");
      const length = takeU32("copy length");
      if (length === 0 || offset + length > baseline.byteLength) {
        fail("program patch copy escapes the baseline");
      }
      if (outputOffset + length > output.byteLength) fail("program patch exceeds its output");
      output.set(baseline.subarray(offset, offset + length), outputOffset);
      outputOffset += length;
    } else if (tag === PATCH_INSERT) {
      const length = takeU32("insert length");
      if (length === 0 || cursor + length > patch.byteLength) {
        fail("program patch insert is invalid");
      }
      if (outputOffset + length > output.byteLength) fail("program patch exceeds its output");
      output.set(patch.subarray(cursor, cursor + length), outputOffset);
      cursor += length;
      outputOffset += length;
    } else {
      fail(`program patch instruction ${index} has an unknown tag`);
    }
  }
  if (cursor !== patch.byteLength || outputOffset !== output.byteLength) {
    fail("program patch length does not match its output contract");
  }
  return output;
}

/** Decode the sealed DrawProgram arena before any CanvasKit object or surface is allocated. */
export async function decodeDrawProgram(input: ArrayBuffer | ArrayBufferView): Promise<DrawProgramWire> {
  const bytes = exactBytes(input);
  if (bytes.byteLength > DRAW_PROGRAM_ABI.maximumBytes) {
    fail(`byte budget exceeded (${bytes.byteLength} > ${DRAW_PROGRAM_ABI.maximumBytes})`);
  }
  if (bytes.byteLength < TABLE_BYTES) fail(`truncated section table (${bytes.byteLength} < ${TABLE_BYTES})`);
  const magic = new TextEncoder().encode(DRAW_PROGRAM_ABI.magic);
  if (!equal(bytes.subarray(0, 8), magic)) fail("wrong magic");
  const view = dataView(bytes);
  const formatVersion = view.getUint32(8, DRAW_LITTLE_ENDIAN);
  if (formatVersion !== DRAW_PROGRAM_ABI.formatVersion) {
    fail(
      `unsupported format version ${formatVersion}; supported version is ${DRAW_PROGRAM_ABI.formatVersion}`,
    );
  }
  if (view.getUint32(12, DRAW_LITTLE_ENDIAN) !== DRAW_PROGRAM_ABI.endiannessMarker) {
    fail("wrong endianness marker");
  }
  if (
    view.getUint16(16, DRAW_LITTLE_ENDIAN) !== SECTION_COUNT
    || view.getUint16(18, DRAW_LITTLE_ENDIAN) !== 0
  ) {
    fail("invalid section-count or reserved header bits");
  }
  if (view.getBigUint64(20, DRAW_LITTLE_ENDIAN) !== BigInt(bytes.byteLength)) {
    fail("declared length mismatch");
  }
  verifyChecksum(bytes);

  const decoder = new TextDecoder("utf-8", { fatal: true });
  const sections: unknown[] = [];
  let expectedOffset = TABLE_BYTES;
  for (let index = 0; index < SECTION_COUNT; index += 1) {
    const start = HEADER_BYTES + index * SECTION_ENTRY_BYTES;
    const kind = view.getUint16(start, DRAW_LITTLE_ENDIAN);
    const flags = view.getUint16(start + 2, DRAW_LITTLE_ENDIAN);
    const count = view.getUint32(start + 4, DRAW_LITTLE_ENDIAN);
    const offset = safeInteger(
      view.getBigUint64(start + 8, DRAW_LITTLE_ENDIAN),
      `section ${kind} offset`,
    );
    const length = safeInteger(
      view.getBigUint64(start + 16, DRAW_LITTLE_ENDIAN),
      `section ${kind} length`,
    );
    if (kind !== EXPECTED_KINDS[index] || flags !== 0 || offset !== expectedOffset) {
      fail(`non-canonical section entry ${index}`);
    }
    const end = offset + length;
    if (!Number.isSafeInteger(end) || end > bytes.byteLength) fail(`section ${kind} is out of range`);
    let value: unknown;
    if (kind === 7) {
      if (length !== count * BATCH_INSTANCE_BYTES) {
        fail(`GeometryBatch section has ${length} bytes for ${count} instances`);
      }
      const batchBytes = bytes.subarray(offset, end);
      value = { bytes: batchBytes, view: dataView(batchBytes), count } satisfies BatchInstanceTableWire;
    } else {
      try {
        value = JSON.parse(decoder.decode(bytes.subarray(offset, end)));
      } catch (cause) {
        fail(`section ${kind} is not canonical UTF-8 JSON: ${errorText(cause)}`);
      }
      if ([1, 2, 3, 4].includes(kind) && (!Array.isArray(value) || value.length !== count)) {
        fail(`section ${kind} count mismatch`);
      }
      if ([5, 6].includes(kind) && count !== 1) fail(`section ${kind} must contain one value`);
    }
    sections.push(value);
    expectedOffset = end;
  }
  if (expectedOffset !== bytes.byteLength) fail("trailing bytes are not owned by a section");

  const [roots, nodes, batchInstances, paths, paints, viewport, requirements] = sections;
  if (!Array.isArray(roots) || !roots.every(positiveIntegerOrZero)) fail("roots are invalid");
  if (!Array.isArray(nodes) || !nodes.every(isRecord)) fail("nodes are invalid");
  if (!isBatchInstanceTable(batchInstances)) fail("GeometryBatch instance table is invalid");
  validateGeometryBatches(nodes as TaggedWire[], batchInstances);
  if (!Array.isArray(paths) || !paths.every(isPath)) fail("paths are invalid");
  if (!Array.isArray(paints) || !paints.every(isRecord)) fail("paints are invalid");
  if (!isRect(viewport) || viewport.width <= 0 || viewport.height <= 0) fail("viewport is invalid");
  if (!isRecord(requirements)) fail("requirements are invalid");
  return {
    roots: roots as number[],
    nodes: nodes as TaggedWire[],
    batchInstances,
    paths: paths as PathWire[],
    paints: paints as TaggedWire[],
    viewport,
    requirements,
  };
}

function isBatchInstanceTable(value: unknown): value is BatchInstanceTableWire {
  return isRecord(value)
    && value.bytes instanceof Uint8Array
    && value.view instanceof DataView
    && positiveIntegerOrZero(value.count)
    && value.bytes.byteLength === value.count * BATCH_INSTANCE_BYTES;
}

function validateGeometryBatches(nodes: TaggedWire[], table: BatchInstanceTableWire): void {
  let consumed = 0;
  for (const [nodeIndex, node] of nodes.entries()) {
    if (node.kind !== "geometryBatch") continue;
    const value = exactRecord(node.value, ["geometry", "instances"], `nodes[${nodeIndex}].value`);
    if (value.geometry !== "circle" && value.geometry !== "rect") {
      fail(`nodes[${nodeIndex}].value.geometry is invalid`);
    }
    const range = exactRecord(
      value.instances,
      ["start", "count"],
      `nodes[${nodeIndex}].value.instances`,
    );
    const start = exactNonNegativeInteger(range.start, `nodes[${nodeIndex}].instances.start`);
    const count = exactNonNegativeInteger(range.count, `nodes[${nodeIndex}].instances.count`);
    if (start !== consumed || count > table.count - consumed) {
      fail("GeometryBatch ranges do not canonically partition the instance table");
    }
    consumed += count;
  }
  if (consumed !== table.count) fail("GeometryBatch instance table has unreferenced values");

  for (let index = 0; index < table.count; index += 1) {
    const offset = index * BATCH_INSTANCE_BYTES;
    const positionX = table.view.getFloat64(offset, DRAW_LITTLE_ENDIAN);
    const positionY = table.view.getFloat64(offset + 8, DRAW_LITTLE_ENDIAN);
    const sizeX = table.view.getFloat64(offset + 16, DRAW_LITTLE_ENDIAN);
    const sizeY = table.view.getFloat64(offset + 24, DRAW_LITTLE_ENDIAN);
    const red = table.view.getFloat32(offset + 32, DRAW_LITTLE_ENDIAN);
    const green = table.view.getFloat32(offset + 36, DRAW_LITTLE_ENDIAN);
    const blue = table.view.getFloat32(offset + 40, DRAW_LITTLE_ENDIAN);
    const alpha = table.view.getFloat32(offset + 44, DRAW_LITTLE_ENDIAN);
    if (![positionX, positionY, sizeX, sizeY, red, green, blue, alpha].every(Number.isFinite)) {
      fail(`GeometryBatch instance ${index} contains a non-finite value`);
    }
    if (sizeX <= 0 || sizeY <= 0) fail(`GeometryBatch instance ${index} has a non-positive size`);
    if (alpha < 0 || alpha > 1) fail(`GeometryBatch instance ${index} alpha is outside [0,1]`);
    if (alpha === 0 && (red !== 0 || green !== 0 || blue !== 0)) {
      fail(`GeometryBatch instance ${index} has RGB under transparent premultiplied alpha`);
    }
  }
}

function exactRecord(value: unknown, keys: string[], label: string): Record<string, unknown> {
  if (!isRecord(value)) fail(`${label} must be an object`);
  const actual = Object.keys(value);
  if (actual.length !== keys.length || actual.some((key, index) => key !== keys[index])) {
    fail(`${label} fields are not canonical`);
  }
  return value;
}

function exactNonNegativeInteger(value: unknown, label: string): number {
  if (!positiveIntegerOrZero(value)) fail(`${label} must be an exact non-negative integer`);
  return value;
}

function verifyChecksum(bytes: Uint8Array): void {
  const copy = bytes.slice();
  const checksumEnd = DRAW_PROGRAM_ABI.checksumOffset + DRAW_PROGRAM_ABI.checksumBytes;
  const expected = copy.slice(DRAW_PROGRAM_ABI.checksumOffset, checksumEnd);
  copy.fill(0, DRAW_PROGRAM_ABI.checksumOffset, checksumEnd);
  const actual = sha256(copy);
  if (!equal(expected, actual)) fail("checksum mismatch");
}

function exactBytes(input: ArrayBuffer | ArrayBufferView): Uint8Array {
  if (input instanceof ArrayBuffer) return new Uint8Array(input);
  return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
}

function dataView(bytes: Uint8Array): DataView {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

function safeInteger(value: bigint, label: string): number {
  const number = Number(value);
  if (!Number.isSafeInteger(number)) fail(`${label} exceeds JavaScript's exact integer range`);
  return number;
}

function equal(left: Uint8Array, right: Uint8Array): boolean {
  return left.byteLength === right.byteLength && left.every((byte, index) => byte === right[index]);
}

function positiveIntegerOrZero(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) >= 0;
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function isRect(value: unknown): value is RectWire {
  return isRecord(value)
    && [value.x, value.y, value.width, value.height].every((part) => typeof part === "number" && Number.isFinite(part));
}

function isPath(value: unknown): value is PathWire {
  return isRecord(value)
    && Array.isArray(value.verbs)
    && value.verbs.every((verb) => typeof verb === "string")
    && Array.isArray(value.points)
    && value.points.every((point) => Array.isArray(point)
      && point.length === 2
      && point.every((part) => typeof part === "number" && Number.isFinite(part)));
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function fail(message: string): never {
  throw new PackedDrawProgramError(message);
}
