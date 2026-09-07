import { describe, expect, test } from "bun:test";

import { PackedAbiError, RENDER_PLAN_ABI, decodePackedAbi } from "./packed.ts";
import { sha256 } from "./sha256.ts";

describe("packed compositor ABI", () => {
  test("validates and decodes one canonical envelope", async () => {
    const payload = Uint8Array.from([
      8, 2, 0, 0, 0, // object(2)
      1, 0, 0, 0, 0x61, // "a"
      6, 2, 0, 0, 0, 0x6f, 0x6b, // string("ok")
      1, 0, 0, 0, 0x6e, // "n"
      3, 7, 0, 0, 0, 0, 0, 0, 0, // i64(7)
    ]);
    const decoded = await decodePackedAbi(await envelope(payload), RENDER_PLAN_ABI);
    expect(decoded).toEqual({ a: "ok", n: 7 });
  });

  test("rejects corruption before decoding values", async () => {
    const bytes = await envelope(Uint8Array.of(0));
    bytes[bytes.length - 1] ^= 1;
    await expect(decodePackedAbi(bytes, RENDER_PLAN_ABI)).rejects.toThrow("checksum mismatch");
  });

  test("rejects the wrong exact u32 format version even with a valid checksum", async () => {
    const bytes = await envelope(Uint8Array.of(0));
    new DataView(bytes.buffer).setUint32(8, RENDER_PLAN_ABI.formatVersion + 1, true);
    seal(bytes);

    await expect(decodePackedAbi(bytes, RENDER_PLAN_ABI)).rejects.toThrow(
      `unsupported format version ${RENDER_PLAN_ABI.formatVersion + 1}`,
    );
  });

  test("rejects an oversized declared container before growing it", async () => {
    const payload = Uint8Array.from([7, 0xff, 0xff, 0xff, 0xff]);
    await expect(decodePackedAbi(await envelope(payload), RENDER_PLAN_ABI)).rejects.toBeInstanceOf(PackedAbiError);
  });

  test("rejects a U64 spelling when the canonical Rust encoder uses I64", async () => {
    const payload = Uint8Array.from([4, 7, 0, 0, 0, 0, 0, 0, 0]);
    await expect(decodePackedAbi(await envelope(payload), RENDER_PLAN_ABI)).rejects.toThrow(
      "non-canonical U64",
    );
  });
});

async function envelope(payload: Uint8Array): Promise<Uint8Array> {
  const bytes = new Uint8Array(RENDER_PLAN_ABI.headerBytes + payload.byteLength);
  bytes.set(new TextEncoder().encode(RENDER_PLAN_ABI.magic), 0);
  const view = new DataView(bytes.buffer);
  view.setUint32(8, RENDER_PLAN_ABI.formatVersion, true);
  view.setUint32(12, RENDER_PLAN_ABI.endiannessMarker, true);
  view.setBigUint64(16, BigInt(bytes.byteLength), true);
  bytes.set(payload, RENDER_PLAN_ABI.headerBytes);
  seal(bytes);
  return bytes;
}

function seal(bytes: Uint8Array): void {
  const end = RENDER_PLAN_ABI.checksumOffset + RENDER_PLAN_ABI.checksumBytes;
  bytes.fill(0, RENDER_PLAN_ABI.checksumOffset, end);
  bytes.set(sha256(bytes), RENDER_PLAN_ABI.checksumOffset);
}
