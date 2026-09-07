import { describe, expect, test } from "bun:test";

import { loadPackedAbiSources, renderPackedAbi } from "./generate-packed-abi.ts";

describe("Rust to TypeScript packed ABI generation", () => {
  test("is byte deterministic and includes all public packets", async () => {
    const sources = await loadPackedAbiSources();
    const first = renderPackedAbi(sources);
    const second = renderPackedAbi(sources);

    expect(first).toBe(second);
    expect(first).toContain("DRAW_PROGRAM_ABI");
    expect(first).toContain("RENDER_PLAN_ABI");
    expect(first).toContain("RENDER_BINDINGS_ABI");
    expect(first).toContain("RESOURCE_REQUESTS_ABI");
    expect(first).toContain("BOUND_PROGRAM_SCHEDULES_ABI");
    expect(first).toContain("PROGRAM_PATCH_ABI");
  });

  test("changes output when an authoritative Rust layout changes", async () => {
    const sources = await loadPackedAbiSources();
    const baseline = renderPackedAbi(sources);
    const changedRenderPlan = sources.renderPlan.replace(
      /(pub const RENDER_PLAN_FORMAT_VERSION\s*:\s*u32\s*=\s*)1(\s*;)/,
      (_match, prefix: string, suffix: string) => `${prefix}2${suffix}`,
    );
    expect(changedRenderPlan).not.toBe(sources.renderPlan);
    const drifted = renderPackedAbi({
      ...sources,
      renderPlan: changedRenderPlan,
    });

    expect(drifted).not.toBe(baseline);
    expect(drifted).toContain('"formatVersion": 2');
  });

  test("fails closed when a required Rust constant disappears", async () => {
    const sources = await loadPackedAbiSources();
    const missingPatchHeader = sources.programPatch.replace(
      /\bHEADER_BYTES\b/,
      "REMOVED_HEADER_BYTES",
    );
    expect(missingPatchHeader).not.toBe(sources.programPatch);
    const invalid = {
      ...sources,
      programPatch: missingPatchHeader,
    };

    expect(() => renderPackedAbi(invalid)).toThrow("HEADER_BYTES");
  });
});
