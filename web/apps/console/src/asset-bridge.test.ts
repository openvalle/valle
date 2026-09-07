import { describe, expect, test } from "bun:test";

import { bridgeProjectAssetToLibrary } from "./asset-bridge.ts";

const digestHex = (prefix: string, tail: string): string => `${prefix}${tail.repeat(64).slice(prefix.length, 64)}`;

describe("project asset to Library bridge", () => {
  test("returns one canonical content digest only for a unique full match", () => {
    const matching = digestHex("0123456789abcdef", "a");
    const unrelated = digestHex("fedcba9876543210", "b");

    expect(
      bridgeProjectAssetToLibrary("asset_0123456789abcdef", [unrelated, matching]),
    ).toEqual({ status: "matched", contentDigest: `sha256:${matching}` });
  });

  test("reports every ambiguous candidate without choosing the first", () => {
    const first = digestHex("0123456789abcdef", "a");
    const second = digestHex("0123456789abcdef", "b");

    expect(
      bridgeProjectAssetToLibrary("asset_0123456789abcdef", [second, first]),
    ).toEqual({
      status: "ambiguous",
      contentDigests: [`sha256:${first}`, `sha256:${second}`],
    });
  });

  test("does not treat missing or malformed project ids as identities", () => {
    const hash = digestHex("0123456789abcdef", "a");

    expect(bridgeProjectAssetToLibrary("asset_0123456789abcdef", [])).toEqual({ status: "missing" });
    expect(bridgeProjectAssetToLibrary("asset_0123456789abcde", [hash])).toEqual({ status: "unaddressable" });
    expect(bridgeProjectAssetToLibrary("asset_0123456789abcdef0", [hash])).toEqual({ status: "unaddressable" });
    expect(bridgeProjectAssetToLibrary("asset_0123456789ABCDEf", [hash])).toEqual({ status: "unaddressable" });
  });

  test("deduplicates repeated rows but rejects malformed Library identities", () => {
    const matching = digestHex("0123456789abcdef", "a");

    expect(
      bridgeProjectAssetToLibrary("asset_0123456789abcdef", [matching, matching]),
    ).toEqual({ status: "matched", contentDigest: `sha256:${matching}` });
    expect(() => bridgeProjectAssetToLibrary("asset_0123456789abcdef", ["0123"])).toThrow(
      "library list contains an invalid content hash",
    );
  });
});
