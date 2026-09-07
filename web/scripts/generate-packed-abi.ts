import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const WEB_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const REPO_ROOT = resolve(WEB_ROOT, "..");

const SOURCE_PATHS = Object.freeze({
  drawPacked: "crates/valle-draw/src/program/packed.rs",
  drawValidate: "crates/valle-draw/src/program/validate.rs",
  enginePacked: "crates/valle-engine/src/compositor/lower/packed.rs",
  renderPlan: "crates/valle-engine/src/compositor/lower/plan.rs",
  renderBindings: "crates/valle-engine/src/compositor/lower/bindings.rs",
  resourceRequests: "crates/valle-engine/src/resource/execution.rs",
  boundSchedules: "crates/valle-engine/src/compositor/lower/program_frame.rs",
  programPatch: "crates/valle-engine/src/compositor/lower/program_patch.rs",
});

export interface PackedAbiSources {
  readonly drawPacked: string;
  readonly drawValidate: string;
  readonly enginePacked: string;
  readonly renderPlan: string;
  readonly renderBindings: string;
  readonly resourceRequests: string;
  readonly boundSchedules: string;
  readonly programPatch: string;
}

export const GENERATED_PACKED_ABI_PATH = resolve(
  WEB_ROOT,
  "packages/engine/src/generated/packed-abi.ts",
);

export async function loadPackedAbiSources(
  repoRoot = REPO_ROOT,
): Promise<PackedAbiSources> {
  const entries = await Promise.all(
    Object.entries(SOURCE_PATHS).map(async ([key, path]) => [
      key,
      await Bun.file(resolve(repoRoot, path)).text(),
    ] as const),
  );
  return Object.fromEntries(entries) as unknown as PackedAbiSources;
}

export function renderPackedAbi(sources: PackedAbiSources): string {
  const drawChecksum = rustChecksumRange(sources.drawPacked, "DrawProgram");
  const engineChecksum = rustRange(sources.enginePacked, "CHECKSUM_RANGE");
  const drawEndianness = rustInteger(sources.drawPacked, "ENDIAN_MARKER");
  const engineEndianness = rustInteger(sources.enginePacked, "ENDIAN_MARKER");
  if (drawEndianness !== engineEndianness) {
    throw new Error(
      `packed ABI endianness markers disagree: DrawProgram=${drawEndianness}, Engine=${engineEndianness}`,
    );
  }

  const engineHeaderBytes = rustInteger(sources.enginePacked, "HEADER_LEN");
  if (engineChecksum.end !== engineHeaderBytes || engineChecksum.end - engineChecksum.start !== 32) {
    throw new Error("Engine packed checksum must be the final 32 bytes of its header");
  }
  const drawHeaderBytes = rustInteger(sources.drawPacked, "HEADER_LEN");
  if (drawChecksum.end !== drawHeaderBytes || drawChecksum.end - drawChecksum.start !== 32) {
    throw new Error("DrawProgram checksum must be the final 32 bytes of its header");
  }

  requireMatch(
    sources.enginePacked,
    /const\s+MAX_STRING_BYTES:\s*usize\s*=\s*MAX_PACKED_PLAN_BYTES\s*;/,
    "MAX_STRING_BYTES alias",
  );

  const model = {
    valueLimits: {
      maximumDepth: rustInteger(sources.enginePacked, "MAX_DEPTH"),
      maximumStringBytes: rustInteger(sources.enginePacked, "MAX_PACKED_PLAN_BYTES"),
      maximumContainerItems: rustInteger(sources.enginePacked, "MAX_CONTAINER_ITEMS"),
      maximumTotalValues: rustInteger(sources.enginePacked, "MAX_TOTAL_VALUES"),
    },
    draw: {
      kind: "DrawProgram",
      magic: rustMagic(sources.drawPacked, "MAGIC"),
      formatVersion: rustInteger(sources.drawPacked, "DRAW_PROGRAM_FORMAT_VERSION"),
      headerBytes: drawHeaderBytes,
      checksumOffset: drawChecksum.start,
      checksumBytes: drawChecksum.end - drawChecksum.start,
      endianness: "little",
      endiannessMarker: drawEndianness,
      maximumBytes: rustInteger(sources.drawValidate, "MAX_PACKED_BYTES"),
      sectionEntryBytes: rustInteger(sources.drawPacked, "ENTRY_LEN"),
      sectionCount: rustInteger(sources.drawPacked, "SECTION_COUNT"),
      batchInstanceBytes: rustInteger(sources.drawPacked, "BATCH_INSTANCE_BYTES"),
    },
    renderPlan: envelopeModel(
      "RenderPlanTemplate",
      rustMagic(sources.enginePacked, "PLAN_MAGIC"),
      rustInteger(sources.renderPlan, "RENDER_PLAN_FORMAT_VERSION"),
      rustInteger(sources.enginePacked, "MAX_PACKED_PLAN_BYTES"),
      engineHeaderBytes,
      engineChecksum,
      engineEndianness,
    ),
    renderBindings: envelopeModel(
      "RenderBindings",
      rustMagic(sources.enginePacked, "BINDINGS_MAGIC"),
      rustInteger(sources.renderBindings, "RENDER_BINDINGS_FORMAT_VERSION"),
      rustInteger(sources.enginePacked, "MAX_PACKED_BINDINGS_BYTES"),
      engineHeaderBytes,
      engineChecksum,
      engineEndianness,
    ),
    resourceRequests: envelopeModel(
      "ResourceRequestSet",
      rustMagic(sources.enginePacked, "REQUESTS_MAGIC"),
      rustInteger(sources.resourceRequests, "RESOURCE_REQUESTS_FORMAT_VERSION"),
      rustInteger(sources.enginePacked, "MAX_PACKED_REQUESTS_BYTES"),
      engineHeaderBytes,
      engineChecksum,
      engineEndianness,
    ),
    boundSchedules: envelopeModel(
      "BoundProgramSchedules",
      rustMagic(sources.enginePacked, "SCHEDULE_MAGIC"),
      rustInteger(sources.boundSchedules, "BOUND_PROGRAM_SCHEDULES_FORMAT_VERSION"),
      rustInteger(sources.enginePacked, "MAX_PACKED_SCHEDULE_BYTES"),
      engineHeaderBytes,
      engineChecksum,
      engineEndianness,
    ),
    programPatch: {
      kind: "ProgramPatch",
      magic: rustMagic(sources.programPatch, "MAGIC"),
      headerBytes: rustInteger(sources.programPatch, "HEADER_BYTES"),
      endianness: "little",
      maximumBytes: rustInteger(sources.programPatch, "MAX_BYTES"),
    },
  } as const;

  return `// @generated by web/scripts/generate-packed-abi.ts from the authoritative Rust layouts.
// Do not edit by hand. Run \`bun run generate:packed-abi\`.

export interface PackedAbiDescriptor {
  readonly kind: string;
  readonly magic: string;
  readonly formatVersion: number;
  readonly headerBytes: number;
  readonly checksumOffset: number;
  readonly checksumBytes: number;
  readonly endianness: "little";
  readonly endiannessMarker: number;
  readonly maximumBytes: number;
}

export interface ProgramPatchAbiDescriptor {
  readonly kind: "ProgramPatch";
  readonly magic: string;
  readonly headerBytes: number;
  readonly endianness: "little";
  readonly maximumBytes: number;
}

export const PACKED_VALUE_LIMITS = Object.freeze(${literal(model.valueLimits)});

export const DRAW_PROGRAM_ABI = Object.freeze(${literal(model.draw)});

export const RENDER_PLAN_ABI = Object.freeze(${literal(model.renderPlan)});

export const RENDER_BINDINGS_ABI = Object.freeze(${literal(model.renderBindings)});

export const RESOURCE_REQUESTS_ABI = Object.freeze(${literal(model.resourceRequests)});

export const BOUND_PROGRAM_SCHEDULES_ABI = Object.freeze(${literal(model.boundSchedules)});

export const PROGRAM_PATCH_ABI = Object.freeze(${literal(model.programPatch)});
`;
}

function envelopeModel(
  kind: string,
  magic: string,
  formatVersion: number,
  maximumBytes: number,
  headerBytes: number,
  checksum: { readonly start: number; readonly end: number },
  endiannessMarker: number,
) {
  return {
    kind,
    magic,
    formatVersion,
    headerBytes,
    checksumOffset: checksum.start,
    checksumBytes: checksum.end - checksum.start,
    endianness: "little",
    endiannessMarker,
    maximumBytes,
  } as const;
}

function rustInteger(source: string, name: string): number {
  const match = requireMatch(
    source,
    new RegExp(`(?:pub(?:\\(crate\\))?\\s+)?const\\s+${name}\\s*:\\s*(?:u32|u64|usize)\\s*=\\s*([^;]+);`),
    name,
  );
  return evaluateInteger(match[1]!.trim(), name);
}

function evaluateInteger(expression: string, label: string): number {
  const factors = expression.split("*").map((part) => part.trim());
  if (factors.some((part) => !/^(?:0x[\da-fA-F_]+|[\d_]+)$/.test(part))) {
    throw new Error(`unsupported Rust integer expression for ${label}: ${expression}`);
  }
  const value = factors.reduce((product, factor) => {
    const normalized = factor.replaceAll("_", "");
    const parsed = normalized.startsWith("0x")
      ? Number.parseInt(normalized.slice(2), 16)
      : Number.parseInt(normalized, 10);
    return product * parsed;
  }, 1);
  if (!Number.isSafeInteger(value)) throw new Error(`${label} is outside JavaScript's exact integer range`);
  return value;
}

function rustMagic(source: string, name: string): string {
  const match = requireMatch(
    source,
    new RegExp(
      `(?:pub(?:\\(crate\\))?\\s+)?const\\s+${name}\\s*:\\s*(?:&)?\\[u8;\\s*8\\]\\s*=\\s*\\*?b"((?:\\\\.|[^"\\\\])*)";`,
    ),
    name,
  );
  const decoded = decodeRustByteString(match[1]!);
  if (new TextEncoder().encode(decoded).byteLength !== 8) {
    throw new Error(`${name} must contain exactly eight bytes`);
  }
  return decoded;
}

function decodeRustByteString(value: string): string {
  return value.replace(/\\(x[\da-fA-F]{2}|0|n|r|t|\\|")/g, (_escape, code: string) => {
    if (code.startsWith("x")) return String.fromCharCode(Number.parseInt(code.slice(1), 16));
    if (code === "0") return "\0";
    if (code === "n") return "\n";
    if (code === "r") return "\r";
    if (code === "t") return "\t";
    return code;
  });
}

function rustRange(source: string, name: string): { start: number; end: number } {
  const match = requireMatch(
    source,
    new RegExp(`const\\s+${name}\\s*:\\s*std::ops::Range<usize>\\s*=\\s*(\\d+)\\s*\\.\\.\\s*(\\d+)\\s*;`),
    name,
  );
  return { start: Number(match[1]), end: Number(match[2]) };
}

function rustChecksumRange(source: string, label: string): { start: number; end: number } {
  const match = requireMatch(
    source,
    /wire\[(\d+)\s*\.\.\s*(\d+)\]\.copy_from_slice\(&checksum\)/,
    `${label} checksum range`,
  );
  return { start: Number(match[1]), end: Number(match[2]) };
}

function requireMatch(source: string, pattern: RegExp, label: string): RegExpMatchArray {
  const match = source.match(pattern);
  if (!match) throw new Error(`could not find authoritative Rust packed ABI constant: ${label}`);
  return match;
}

function literal(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

async function main(): Promise<void> {
  const mode = Bun.argv[2];
  if (mode !== "--write" && mode !== "--check") {
    throw new Error("usage: bun run scripts/generate-packed-abi.ts --write|--check");
  }
  const output = renderPackedAbi(await loadPackedAbiSources());
  if (mode === "--write") {
    await Bun.write(GENERATED_PACKED_ABI_PATH, output);
    console.log("generated packages/engine/src/generated/packed-abi.ts");
    return;
  }
  const current = await Bun.file(GENERATED_PACKED_ABI_PATH).text().catch(() => "");
  if (current !== output) {
    throw new Error(
      "packages/engine/src/generated/packed-abi.ts drifted; run bun run generate:packed-abi",
    );
  }
  console.log("packed ABI generation: checked");
}

if (import.meta.main) await main();
