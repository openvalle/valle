import { describe, expect, test } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { STUDIO_HOST_PROTOCOL_VERSION } from "../packages/engine/src/generated/protocol.ts";
import {
  assertCanvasKitBuildVersion,
  assertGeneratedEngineBuildVersion,
  readRuntimeProtocolVersion,
  readValleWorkspaceVersion,
} from "./runtime-version.ts";

describe("Valle Web runtime build version", () => {
  test("rejects an installed CanvasKit package that disagrees with the declared pin", async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "valle-canvaskit-version-"));
    try {
      await writeFile(path.join(root, "package.json"), JSON.stringify({ version: "999.0.0" }));
      await expect(assertCanvasKitBuildVersion(root)).rejects.toThrow("does not match workspace catalog");
      const declared = await Bun.file(path.resolve(import.meta.dir, "../package.json")).json();
      await writeFile(path.join(root, "package.json"), JSON.stringify({ version: declared.workspaces.catalog["canvaskit-wasm"] }));
      await expect(assertCanvasKitBuildVersion(root)).resolves.toBeUndefined();
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
  test("reads the host/runtime protocol from its single source file", async () => {
    const workspaceRoot = path.resolve(import.meta.dir, "../..");
    await expect(readRuntimeProtocolVersion(workspaceRoot)).resolves.toBe(
      STUDIO_HOST_PROTOCOL_VERSION,
    );
  });

  test("reads the Cargo workspace package version used by the root runtime manifest", async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "valle-runtime-version-"));
    try {
      await writeFile(path.join(root, "Cargo.toml"), `
[workspace]
members = []

[workspace.package]
version = "3.4.5"
`);
      await expect(readValleWorkspaceVersion(root)).resolves.toBe("3.4.5");
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("rejects a Cargo workspace without one build version", async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "valle-runtime-version-"));
    try {
      await writeFile(path.join(root, "Cargo.toml"), "[workspace]\nmembers = []\n");
      await expect(readValleWorkspaceVersion(root)).rejects.toThrow(
        "Valle Cargo workspace version is missing",
      );
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("rejects a generated Web engine built under another Cargo version", async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "valle-generated-engine-version-"));
    try {
      await writeFile(path.join(root, "package.json"), JSON.stringify({ version: "3.4.4" }));
      await expect(assertGeneratedEngineBuildVersion(root, "3.4.5")).rejects.toThrow(
        "run build:engine first",
      );
      await writeFile(path.join(root, "package.json"), JSON.stringify({ version: "3.4.5" }));
      await expect(assertGeneratedEngineBuildVersion(root, "3.4.5")).resolves.toBeUndefined();
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
