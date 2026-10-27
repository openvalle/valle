import type { PixelRule } from "./pixels.ts";

interface Run {
  settings: { qa: unknown };
  identities: { cases: Record<string, unknown> };
  qa: { case: string; backend: string; optimizationRule?: PixelRule }[];
}

/** Fail closed before reading pixels: missing captures and a changed tolerance are not a pass. */
export function comparisonCases(before: Run, after: Run, rules: "exact" | "registered") {
  if (JSON.stringify(before.settings.qa) !== JSON.stringify(after.settings.qa))
    throw new Error("QA resolution/schedule differ");
  if (!before.qa?.length || !after.qa?.length) throw new Error("both runs need captured QA rows");
  const coverage = (run: Run) => run.qa.map((row) => `${row.backend}/${row.case}`).sort();
  const a = coverage(before),
    b = coverage(after);
  if (new Set(a).size !== a.length || new Set(b).size !== b.length)
    throw new Error("duplicate case/backend captures");
  if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error("case/backend coverage differs");
  return before.qa.map((previous) => {
    const c = previous.case,
      backend = previous.backend;
    const current = after.qa.find((row) => row.case === c && row.backend === backend)!;
    if (
      !before.identities.cases[c] ||
      !after.identities.cases[c] ||
      JSON.stringify(before.identities.cases[c]) !== JSON.stringify(after.identities.cases[c])
    )
      throw new Error(`fixture identity differs: ${c}`);
    if (
      rules === "registered" &&
      (!previous.optimizationRule ||
        JSON.stringify(previous.optimizationRule) !== JSON.stringify(current.optimizationRule))
    )
      throw new Error(`optimization tolerance missing or changed: ${c}/${backend}`);
    return {
      case: c,
      backend,
      rule: rules === "exact" ? { rgb: 0, alpha: 0 } : current.optimizationRule!,
    };
  });
}
