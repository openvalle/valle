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
const EXPECTED_KINDS = [1, 2, 7, 8, 9, 3, 4, 5, 6] as const;
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
export interface StrokeColorTableWire {
  readonly bytes: Uint8Array;
  readonly view: DataView;
  readonly count: number;
}
export interface DashOffsetTableWire {
  readonly bytes: Uint8Array;
  readonly view: DataView;
  readonly count: number;
}
export interface DrawProgramWire {
  roots: number[];
  nodes: TaggedWire[];
  batchInstances: BatchInstanceTableWire;
  strokeColors: StrokeColorTableWire;
  dashOffsets: DashOffsetTableWire;
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

/** Decode the DrawProgram arena before any CanvasKit object or surface is allocated. */
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
    if (kind === 7 || kind === 8 || kind === 9) {
      const rowBytes = kind === 7 ? BATCH_INSTANCE_BYTES : kind === 8 ? 16 : 4;
      if (length !== count * rowBytes) {
        fail(`InstanceBatch section ${kind} has ${length} bytes for ${count} instances`);
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

  const [roots, nodes, batchInstances, strokeColors, dashOffsets, paths, paints, viewport, requirements] = sections;
  if (!Array.isArray(roots) || !roots.every(positiveIntegerOrZero)) fail("roots are invalid");
  if (!Array.isArray(nodes) || !nodes.every(isRecord)) fail("nodes are invalid");
  if (!isBatchInstanceTable(batchInstances)) fail("InstanceBatch instance table is invalid");
  if (!isStrokeColorTable(strokeColors)) fail("InstanceBatch stroke color table is invalid");
  if (!isDashOffsetTable(dashOffsets)) fail("InstanceBatch dash offset table is invalid");
  if (!Array.isArray(paths) || !paths.every(isPath)) fail("paths are invalid");
  validateInstanceBatches(nodes as TaggedWire[], batchInstances, strokeColors, dashOffsets, paths.length);
  if (!Array.isArray(paints) || !paints.every(isRecord)) fail("paints are invalid");
  if (!isRect(viewport) || viewport.width <= 0 || viewport.height <= 0) fail("viewport is invalid");
  if (!isRecord(requirements)) fail("requirements are invalid");
  return {
    roots: roots as number[],
    nodes: nodes as TaggedWire[],
    batchInstances,
    strokeColors,
    dashOffsets,
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

function isStrokeColorTable(value: unknown): value is StrokeColorTableWire {
  return isRecord(value)
    && value.bytes instanceof Uint8Array
    && value.view instanceof DataView
    && positiveIntegerOrZero(value.count)
    && value.bytes.byteLength === value.count * 16;
}

function isDashOffsetTable(value: unknown): value is DashOffsetTableWire {
  return isRecord(value)
    && value.bytes instanceof Uint8Array
    && value.view instanceof DataView
    && positiveIntegerOrZero(value.count)
    && value.bytes.byteLength === value.count * 4;
}

function validateInstanceBatches(nodes: TaggedWire[], table: BatchInstanceTableWire, strokeColors: StrokeColorTableWire, dashOffsets: DashOffsetTableWire, pathCount: number): void {
  let consumed = 0;
  let colorsConsumed = 0;
  let offsetsConsumed = 0;
  for (const [nodeIndex, node] of nodes.entries()) {
    if (node.kind !== "instanceBatch") continue;
    const value = exactRecord(node.value, ["shape", "instances", "strokeColors", "dashOffsets", "pathStyle"], `nodes[${nodeIndex}].value`);
    const shape = value.shape;
    if (!isRecord(shape)) fail(`nodes[${nodeIndex}].shape must be an object`);
    if (shape.kind === "path") {
      exactRecord(shape, ["kind", "value"], `nodes[${nodeIndex}].shape`);
      const path = exactNonNegativeInteger(shape.value, `nodes[${nodeIndex}].shape.value`);
      if (path >= pathCount) fail(`nodes[${nodeIndex}].shape.value is out of range`);
    } else if (shape.kind === "image") {
      exactRecord(shape, ["kind", "value"], `nodes[${nodeIndex}].shape`);
      const region = exactRecord(shape.value, ["texture", "src", "sampling"], `nodes[${nodeIndex}].shape.value`);
      const texture = exactRecord(region.texture, ["key", "kind", "colorDomain", "alpha", "sampleTimeMicros"], `nodes[${nodeIndex}].shape.value.texture`);
      if (typeof texture.key !== "string" || texture.key.length === 0
        || new TextEncoder().encode(texture.key).length > 4096
        || !["image", "video", "generated"].includes(String(texture.kind))
        || texture.colorDomain !== "linearRec2020"
        || !["opaque", "straight", "premultiplied"].includes(String(texture.alpha))
        || (texture.kind === "video"
          ? !positiveIntegerOrZero(texture.sampleTimeMicros)
          : texture.sampleTimeMicros !== null)
        || !["nearestClamp", "linearClamp", "linearDecal", "cubicClamp"].includes(String(region.sampling))
        || !isRect(region.src) || region.src.x < 0 || region.src.y < 0
        || region.src.width <= 0 || region.src.height <= 0
        || region.src.x + region.src.width > 1 || region.src.y + region.src.height > 1) {
        fail(`nodes[${nodeIndex}].shape.value is invalid`);
      }
    } else if (shape.kind === "roundRect") {
      exactRecord(shape, ["kind", "value"], `nodes[${nodeIndex}].shape`);
      const roundRect = exactRecord(shape.value, ["rect", "radii"], `nodes[${nodeIndex}].shape.value`);
      if (!isRect(roundRect.rect) || roundRect.rect.width < 0 || roundRect.rect.height < 0) {
        fail(`nodes[${nodeIndex}].shape.value.rect is invalid`);
      }
      const bounds = roundRect.rect;
      if ([bounds.x, bounds.y, bounds.width, bounds.height, bounds.x + bounds.width, bounds.y + bounds.height]
        .some((coordinate) => Math.abs(coordinate) > 16_777_216)) {
        fail(`nodes[${nodeIndex}].shape.value.rect exceeds the coordinate limit`);
      }
      if (!Array.isArray(roundRect.radii) || roundRect.radii.length !== 4
        || !roundRect.radii.every((pair) => Array.isArray(pair) && pair.length === 2
          && pair.every((radius) => typeof radius === "number" && Number.isFinite(radius)
            && radius >= 0 && radius <= 16_777_216))) {
        fail(`nodes[${nodeIndex}].shape.value.radii is invalid`);
      }
    } else if (shape.kind === "circle" || shape.kind === "rect") {
      exactRecord(shape, ["kind"], `nodes[${nodeIndex}].shape`);
    } else {
      fail(`nodes[${nodeIndex}].shape.kind is invalid`);
    }
    if (value.pathStyle !== null) {
      if (shape.kind !== "path") fail(`nodes[${nodeIndex}].pathStyle requires a Path shape`);
      const style = exactRecord(value.pathStyle, ["fill", "dash", "dashOffset", "cap", "join", "miterLimit"], `nodes[${nodeIndex}].value.pathStyle`);
      if (typeof style.fill !== "boolean" || !["butt", "round", "square"].includes(String(style.cap))
        || !["miter", "round", "bevel"].includes(String(style.join))
        || typeof style.miterLimit !== "number" || !Number.isFinite(style.miterLimit) || style.miterLimit < 1
        || typeof style.dashOffset !== "number" || !Number.isFinite(style.dashOffset)
        || !Array.isArray(style.dash) || style.dash.length % 2 !== 0
        || !style.dash.every((part: unknown) => typeof part === "number" && Number.isFinite(part) && part >= 0)
        || style.dash.length > 0 && style.dash.reduce((sum: number, part: number) => sum + part, 0) <= 0) {
        fail(`nodes[${nodeIndex}].pathStyle is invalid`);
      }
    }
    const range = exactRecord(
      value.instances,
      ["start", "count"],
      `nodes[${nodeIndex}].value.instances`,
    );
    const start = exactNonNegativeInteger(range.start, `nodes[${nodeIndex}].instances.start`);
    const count = exactNonNegativeInteger(range.count, `nodes[${nodeIndex}].instances.count`);
    if (start !== consumed || count > table.count - consumed) {
      fail("InstanceBatch ranges do not canonically partition the instance table");
    }
    if (shape.kind === "image") {
      for (let row = start; row < start + count; row += 1) {
        if (table.view.getFloat32(table.count * 68 + row * 4, DRAW_LITTLE_ENDIAN) !== 0) {
          fail(`nodes[${nodeIndex}].image instance ${row} has a stroke`);
        }
      }
    }
    consumed += count;
    if (value.strokeColors !== null) {
      const colors = exactRecord(value.strokeColors, ["start", "count"], `nodes[${nodeIndex}].value.strokeColors`);
      const colorStart = exactNonNegativeInteger(colors.start, `nodes[${nodeIndex}].strokeColors.start`);
      const colorCount = exactNonNegativeInteger(colors.count, `nodes[${nodeIndex}].strokeColors.count`);
      if (colorStart !== colorsConsumed || colorCount !== count || colorCount > strokeColors.count - colorsConsumed) {
        fail("InstanceBatch stroke color ranges do not canonically partition the color table");
      }
      colorsConsumed += colorCount;
    }
    if (value.dashOffsets !== null) {
      if (value.pathStyle === null) fail(`nodes[${nodeIndex}].dashOffsets requires a Path style`);
      const offsets = exactRecord(value.dashOffsets, ["start", "count"], `nodes[${nodeIndex}].value.dashOffsets`);
      const offsetStart = exactNonNegativeInteger(offsets.start, `nodes[${nodeIndex}].dashOffsets.start`);
      const offsetCount = exactNonNegativeInteger(offsets.count, `nodes[${nodeIndex}].dashOffsets.count`);
      if (offsetStart !== offsetsConsumed || offsetCount !== count || offsetCount > dashOffsets.count - offsetsConsumed) {
        fail("InstanceBatch dash ranges do not canonically partition the offset table");
      }
      offsetsConsumed += offsetCount;
    }
  }
  if (consumed !== table.count) fail("InstanceBatch instance table has unreferenced values");
  if (colorsConsumed !== strokeColors.count) fail("InstanceBatch stroke color table has unreferenced values");
  if (offsetsConsumed !== dashOffsets.count) fail("InstanceBatch dash offset table has unreferenced values");

  for (let index = 0; index < table.count; index += 1) {
    const transform = Array.from({ length: 6 }, (_, slot) => table.view.getFloat64(index * 48 + slot * 8, DRAW_LITTLE_ENDIAN));
    const colorOffset = table.count * 48 + index * 16;
    const red = table.view.getFloat32(colorOffset, DRAW_LITTLE_ENDIAN);
    const green = table.view.getFloat32(colorOffset + 4, DRAW_LITTLE_ENDIAN);
    const blue = table.view.getFloat32(colorOffset + 8, DRAW_LITTLE_ENDIAN);
    const alpha = table.view.getFloat32(colorOffset + 12, DRAW_LITTLE_ENDIAN);
    const opacity = table.view.getFloat32(table.count * 64 + index * 4, DRAW_LITTLE_ENDIAN);
    const stroke = table.view.getFloat32(table.count * 68 + index * 4, DRAW_LITTLE_ENDIAN);
    if (![...transform, red, green, blue, alpha, opacity, stroke].every(Number.isFinite)) {
      fail(`InstanceBatch instance ${index} contains a non-finite value`);
    }
    if (transform.some((value) => Math.abs(value) > 16_777_216)) {
      fail(`InstanceBatch instance ${index} exceeds the coordinate limit`);
    }
    const determinant = transform[0]! * transform[3]! - transform[1]! * transform[2]!;
    if (!Number.isFinite(determinant) || determinant === 0 || !Number.isFinite(1 / determinant)) {
      fail(`InstanceBatch instance ${index} has a singular transform`);
    }
    if (alpha < 0 || alpha > 1) fail(`InstanceBatch instance ${index} alpha is outside [0,1]`);
    if (opacity < 0 || opacity > 1) fail(`InstanceBatch instance ${index} opacity is outside [0,1]`);
    if (stroke < 0) fail(`InstanceBatch instance ${index} stroke width is negative`);
    if (alpha === 0 && (red !== 0 || green !== 0 || blue !== 0)) {
      fail(`InstanceBatch instance ${index} has RGB under transparent premultiplied alpha`);
    }
  }
  for (let index = 0; index < strokeColors.count; index += 1) {
    const offset = index * 16;
    const channels = Array.from({ length: 4 }, (_, slot) => strokeColors.view.getFloat32(offset + slot * 4, DRAW_LITTLE_ENDIAN));
    if (!channels.every(Number.isFinite) || channels[3]! < 0 || channels[3]! > 1) {
      fail(`InstanceBatch stroke color ${index} is invalid`);
    }
    if (channels[3] === 0 && channels.slice(0, 3).some((value) => value !== 0)) {
      fail(`InstanceBatch stroke color ${index} has RGB under transparent premultiplied alpha`);
    }
  }
  for (let index = 0; index < dashOffsets.count; index += 1) {
    if (!Number.isFinite(dashOffsets.view.getFloat32(index * 4, DRAW_LITTLE_ENDIAN))) {
      fail(`InstanceBatch dash offset ${index} is non-finite`);
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
