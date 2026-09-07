import path from "node:path";
import workspacePackage from "../package.json";

/** Verify the bytes being packaged come from the dependency version reported by both hosts. */
export async function assertCanvasKitBuildVersion(installedRoot: string): Promise<void> {
  const installed = await Bun.file(path.join(installedRoot, "package.json")).json();
  const expected = workspacePackage.workspaces.catalog["canvaskit-wasm"];
  if (installed.version !== expected) {
    throw new Error(`installed CanvasKit '${String(installed.version)}' does not match workspace catalog '${expected}'; run bun install`);
  }
}

/**
 * Product build version recorded once on the root Web runtime manifest.
 */
export async function readValleWorkspaceVersion(workspaceRoot: string): Promise<string> {
  const cargoToml = path.join(workspaceRoot, "Cargo.toml");
  const cargo = Bun.TOML.parse(await Bun.file(cargoToml).text()) as {
    workspace?: { package?: { version?: unknown } };
  };
  const version = cargo.workspace?.package?.version;
  if (typeof version !== "string" || version.length === 0) {
    throw new Error(`Valle Cargo workspace version is missing from ${cargoToml}`);
  }
  return version;
}

/** Exact host-to-runtime compatibility protocol sourced from one repository file. */
export async function readRuntimeProtocolVersion(workspaceRoot: string): Promise<number> {
  const source = path.join(workspaceRoot, "web", "runtime-protocol-version.txt");
  const text = (await Bun.file(source).text()).trim();
  if (!/^[1-9][0-9]*$/.test(text)) {
    throw new Error(`Valle Web runtime protocol version is invalid in ${source}`);
  }
  const version = Number(text);
  if (!Number.isSafeInteger(version)) {
    throw new Error(`Valle Web runtime protocol version is outside the safe integer range in ${source}`);
  }
  return version;
}

/** Reject copying a generated WASM package produced under another Cargo build version. */
export async function assertGeneratedEngineBuildVersion(
  generatedEngineRoot: string,
  expectedVersion: string,
): Promise<void> {
  const packageJson = path.join(generatedEngineRoot, "package.json");
  const manifest = JSON.parse(await Bun.file(packageJson).text()) as { version?: unknown };
  if (manifest.version !== expectedVersion) {
    throw new Error(
      `generated Web engine version '${String(manifest.version)}' does not match Cargo workspace '${expectedVersion}'; run build:engine first`,
    );
  }
}
