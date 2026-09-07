import { describe, expect, test } from "bun:test";
import path from "node:path";

import {
  assertRuntimeManifest,
  type RootRuntimeManifest,
} from "./runtime-manifest.ts";
import { readRuntimeProtocolVersion, readValleWorkspaceVersion } from "./runtime-version.ts";

const root = path.resolve(import.meta.dir, "..");
const dist = path.join(root, "dist");
const manifest = JSON.parse(
  await Bun.file(path.join(dist, "runtime", "manifest.json")).text(),
) as RootRuntimeManifest;
const valleBuildVersion = await readValleWorkspaceVersion(path.resolve(root, ".."));
const runtimeProtocolVersion = await readRuntimeProtocolVersion(path.resolve(root, ".."));

describe("root runtime manifest closure", () => {
  test("uses the Cargo workspace build version on the one root manifest", () => {
    expect(manifest.runtimeVersion).toBe(valleBuildVersion);
    expect(manifest.protocolVersion).toBe(runtimeProtocolVersion);
  });

  test("verifies every path, hash, license and glue+wasm group", async () => {
    await expect(assertRuntimeManifest(dist, manifest)).resolves.toBeUndefined();
  });

  test("rejects a glue path outside its declared closure", async () => {
    const splitGroup = structuredClone(manifest);
    splitGroup.assetGroups[0].glue = manifest.runtimeAssets.fonts.defaultSans;
    await expect(assertRuntimeManifest(dist, splitGroup)).rejects.toThrow(/outside its glue\+wasm group/);
  });

  test("rejects missing license metadata and byte drift", async () => {
    const missingLicense = structuredClone(manifest);
    missingLicense.assets[0].license = "";
    await expect(assertRuntimeManifest(dist, missingLicense)).rejects.toThrow(/license is required/);

    const hashDrift = structuredClone(manifest);
    hashDrift.assets[0].sha256 = "0".repeat(64);
    await expect(assertRuntimeManifest(dist, hashDrift)).rejects.toThrow(/hash\/size drift/);
  });

  test("rejects an invalid runtime protocol", async () => {
    const invalidProtocol = structuredClone(manifest);
    invalidProtocol.protocolVersion = 0;
    await expect(assertRuntimeManifest(dist, invalidProtocol)).rejects.toThrow(/protocolVersion is invalid/);
  });
});
