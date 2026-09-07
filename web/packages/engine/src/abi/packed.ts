import { sha256 } from "./sha256.ts";
import {
  BOUND_PROGRAM_SCHEDULES_ABI,
  PACKED_VALUE_LIMITS,
  RENDER_BINDINGS_ABI,
  RENDER_PLAN_ABI,
  RESOURCE_REQUESTS_ABI,
  type PackedAbiDescriptor,
} from "../generated/packed-abi.ts";

export {
  BOUND_PROGRAM_SCHEDULES_ABI,
  RENDER_BINDINGS_ABI,
  RENDER_PLAN_ABI,
  RESOURCE_REQUESTS_ABI,
};

const {
  maximumDepth: MAX_DEPTH,
  maximumStringBytes: MAX_STRING_BYTES,
  maximumContainerItems: MAX_CONTAINER_ITEMS,
  maximumTotalValues: MAX_TOTAL_VALUES,
} = PACKED_VALUE_LIMITS;

const TAG_NULL = 0;
const TAG_FALSE = 1;
const TAG_TRUE = 2;
const TAG_I64 = 3;
const TAG_U64 = 4;
const TAG_F64 = 5;
const TAG_STRING = 6;
const TAG_ARRAY = 7;
const TAG_OBJECT = 8;

export type PackedAbiContract = PackedAbiDescriptor;

export type PackedValue =
  | null
  | boolean
  | number
  | bigint
  | string
  | readonly PackedValue[]
  | { readonly [key: string]: PackedValue };

type Node =
  | { tag: typeof TAG_NULL }
  | { tag: typeof TAG_FALSE | typeof TAG_TRUE }
  | { tag: typeof TAG_I64 | typeof TAG_U64; value: bigint }
  | { tag: typeof TAG_F64; value: number }
  | { tag: typeof TAG_STRING; value: string }
  | { tag: typeof TAG_ARRAY; values: Node[] }
  | { tag: typeof TAG_OBJECT; entries: Array<readonly [string, Node]> };

/**
 * Decode the canonical product ABI before an executor allocates surfaces or mutates a target.
 * The returned object contains only data; CanvasKit/WebCodecs objects remain in the host table.
 */
export async function decodePackedAbi(
  input: ArrayBuffer | ArrayBufferView,
  contract: PackedAbiContract,
): Promise<PackedValue> {
  const bytes = exactBytes(input);
  validateEnvelopeShape(bytes, contract);
  validateChecksum(bytes, contract);

  const cursor = new Cursor(bytes.subarray(contract.headerBytes), contract);
  const node = cursor.node(0);
  if (!cursor.done) throw new PackedAbiError(contract.kind, "trailing payload bytes");
  return materialize(node);
}

export class PackedAbiError extends Error {
  constructor(readonly kind: string, message: string) {
    super(`packed ${kind}: ${message}`);
    this.name = "PackedAbiError";
  }
}

function validateEnvelopeShape(bytes: Uint8Array, contract: PackedAbiContract): void {
  if (bytes.byteLength > contract.maximumBytes) {
    throw new PackedAbiError(contract.kind, `byte budget exceeded (${bytes.byteLength} > ${contract.maximumBytes})`);
  }
  if (bytes.byteLength < contract.headerBytes) {
    throw new PackedAbiError(
      contract.kind,
      `truncated header (${bytes.byteLength} < ${contract.headerBytes})`,
    );
  }
  const expectedMagic = new TextEncoder().encode(contract.magic);
  if (expectedMagic.byteLength !== 8 || !equalBytes(bytes.subarray(0, 8), expectedMagic)) {
    throw new PackedAbiError(contract.kind, "wrong magic");
  }
  const view = dataView(bytes);
  const littleEndian = contract.endianness === "little";
  const formatVersion = view.getUint32(8, littleEndian);
  if (formatVersion !== contract.formatVersion) {
    throw new PackedAbiError(
      contract.kind,
      `unsupported format version ${formatVersion}; expected ${contract.formatVersion}`,
    );
  }
  if (view.getUint32(12, littleEndian) !== contract.endiannessMarker) {
    throw new PackedAbiError(contract.kind, "wrong endianness marker");
  }
  const declared = view.getBigUint64(16, littleEndian);
  if (declared !== BigInt(bytes.byteLength)) {
    throw new PackedAbiError(contract.kind, `length mismatch (${declared} != ${bytes.byteLength})`);
  }
}

function validateChecksum(bytes: Uint8Array, contract: PackedAbiContract): void {
  const copy = bytes.slice();
  const checksumEnd = contract.checksumOffset + contract.checksumBytes;
  const expected = copy.slice(contract.checksumOffset, checksumEnd);
  copy.fill(0, contract.checksumOffset, checksumEnd);
  const digest = sha256(copy);
  if (!equalBytes(expected, digest)) throw new PackedAbiError(contract.kind, "checksum mismatch");
}

class Cursor {
  private offset = 0;
  private values = 0;
  private readonly decoder = new TextDecoder("utf-8", { fatal: true });

  constructor(
    private readonly bytes: Uint8Array,
    private readonly contract: PackedAbiContract,
  ) {}

  get done(): boolean {
    return this.offset === this.bytes.byteLength;
  }

  node(depth: number): Node {
    if (depth > MAX_DEPTH) this.fail("value nesting exceeds depth budget");
    this.values += 1;
    if (this.values > MAX_TOTAL_VALUES) this.fail("aggregate value budget exceeded");
    const tag = this.byte();
    switch (tag) {
      case TAG_NULL:
        return { tag };
      case TAG_FALSE:
      case TAG_TRUE:
        return { tag };
      case TAG_I64:
        return { tag, value: this.i64() };
      case TAG_U64: {
        const value = this.u64();
        // Rust encoding probes `as_i64()` before `as_u64()`. Values in the signed range therefore
        // have exactly one representation and must use TAG_I64, even when authored as u64.
        if (value <= 0x7fff_ffff_ffff_ffffn) this.fail("non-canonical U64 in the I64 range");
        return { tag, value };
      }
      case TAG_F64: {
        const bits = this.u64();
        const buffer = new ArrayBuffer(8);
        const view = new DataView(buffer);
        view.setBigUint64(0, bits, true);
        const value = view.getFloat64(0, true);
        if (!Number.isFinite(value) || Object.is(value, -0)) this.fail("non-finite float or negative zero");
        return { tag, value };
      }
      case TAG_STRING: {
        const bytes = this.byteString();
        return { tag, value: this.utf8(bytes) };
      }
      case TAG_ARRAY: {
        const count = this.count();
        const values: Node[] = [];
        for (let index = 0; index < count; index += 1) values.push(this.node(depth + 1));
        return { tag, values };
      }
      case TAG_OBJECT: {
        const count = this.count();
        const entries: Array<readonly [string, Node]> = [];
        let previous: Uint8Array | null = null;
        for (let index = 0; index < count; index += 1) {
          const keyBytes = this.byteString();
          if (previous !== null && compareBytes(previous, keyBytes) >= 0) {
            this.fail("object keys are duplicated or not canonical");
          }
          previous = keyBytes;
          entries.push([this.utf8(keyBytes), this.node(depth + 1)]);
        }
        return { tag, entries };
      }
      default:
        return this.fail(`unknown value tag ${tag}`);
    }
  }

  private byte(): number {
    if (this.offset >= this.bytes.byteLength) this.fail("unexpected end of payload");
    return this.bytes[this.offset++]!;
  }

  private take(length: number): Uint8Array {
    const end = this.offset + length;
    if (!Number.isSafeInteger(end) || end > this.bytes.byteLength) this.fail("value extends beyond payload");
    const result = this.bytes.subarray(this.offset, end);
    this.offset = end;
    return result;
  }

  private u32(): number {
    const bytes = this.take(4);
    return dataView(bytes).getUint32(0, true);
  }

  private i64(): bigint {
    const bytes = this.take(8);
    return dataView(bytes).getBigInt64(0, true);
  }

  private u64(): bigint {
    const bytes = this.take(8);
    return dataView(bytes).getBigUint64(0, true);
  }

  private count(): number {
    const count = this.u32();
    if (count > MAX_CONTAINER_ITEMS) this.fail("container item budget exceeded");
    return count;
  }

  private byteString(): Uint8Array {
    const length = this.u32();
    if (length > MAX_STRING_BYTES) this.fail("string byte budget exceeded");
    return this.take(length);
  }

  private utf8(bytes: Uint8Array): string {
    try {
      return this.decoder.decode(bytes);
    } catch {
      return this.fail("string is not UTF-8");
    }
  }

  private fail(message: string): never {
    throw new PackedAbiError(this.contract.kind, message);
  }
}

function materialize(node: Node): PackedValue {
  switch (node.tag) {
    case TAG_NULL:
      return null;
    case TAG_FALSE:
      return false;
    case TAG_TRUE:
      return true;
    case TAG_I64:
    case TAG_U64:
      return node.value >= BigInt(Number.MIN_SAFE_INTEGER) && node.value <= BigInt(Number.MAX_SAFE_INTEGER)
        ? Number(node.value)
        : node.value;
    case TAG_F64:
      return node.value;
    case TAG_STRING:
      return node.value;
    case TAG_ARRAY:
      return node.values.map(materialize);
    case TAG_OBJECT: {
      const result = Object.create(null) as Record<string, PackedValue>;
      for (const [key, value] of node.entries) result[key] = materialize(value);
      return result;
    }
  }
}

function exactBytes(input: ArrayBuffer | ArrayBufferView): Uint8Array {
  return input instanceof ArrayBuffer
    ? new Uint8Array(input)
    : new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
}

function dataView(bytes: Uint8Array): DataView {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

function equalBytes(left: Uint8Array, right: Uint8Array): boolean {
  if (left.byteLength !== right.byteLength) return false;
  let different = 0;
  for (let index = 0; index < left.byteLength; index += 1) different |= left[index]! ^ right[index]!;
  return different === 0;
}

function compareBytes(left: Uint8Array, right: Uint8Array): number {
  const common = Math.min(left.byteLength, right.byteLength);
  for (let index = 0; index < common; index += 1) {
    const difference = left[index]! - right[index]!;
    if (difference !== 0) return difference;
  }
  return left.byteLength - right.byteLength;
}
