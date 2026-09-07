import { describe, expect, test } from "bun:test";

import {
  assertRuntimeEntrypoints,
  PACKAGE_BUILDS,
} from "./runtime-entrypoints.ts";

describe("runtime build entries", () => {
  test("contains one unique entry for every browser package surface", () => {
    expect(() => assertRuntimeEntrypoints()).not.toThrow();
  });

  test("rejects duplicated entries", () => {
    const duplicated = [...PACKAGE_BUILDS, ["duplicate", [PACKAGE_BUILDS[0][1][0]]] as const];
    expect(() => assertRuntimeEntrypoints(duplicated)).toThrow(/unique/);
  });
});
