import { describe, expect, test } from "bun:test";

import { sha256, sha256Hex } from "./sha256.ts";

describe("synchronous SHA-256", () => {
  test("matches the standard empty and abc vectors", () => {
    expect(sha256Hex(new Uint8Array())).toBe(
      "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    );
    expect(sha256Hex(new TextEncoder().encode("abc"))).toBe(
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
  });

  test("matches WebCrypto across padding boundaries and product-sized input", async () => {
    for (const length of [1, 55, 56, 63, 64, 65, 4_097, 256 * 1024]) {
      const bytes = Uint8Array.from({ length }, (_, index) => (index * 131 + length) & 0xff);
      const expected = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
      expect(sha256(bytes)).toEqual(expected);
    }
  });
});
