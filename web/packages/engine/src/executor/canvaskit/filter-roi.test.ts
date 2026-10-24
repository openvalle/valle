import { expect, test } from "bun:test";
import { cropProgramInputs } from "./filter-roi.ts";

type Kind = Record<string, any>;
const source = { x: -1000, y: -900, width: 4013, height: 3141 };
const output = { x: 0, y: 0, width: 1920, height: 1080 };
const footprint = { x: -16, y: -16, width: 1952, height: 1120 };
const planner = (..._args: unknown[]) =>
  Float64Array.from([-16, -16, 1952, 1120, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0]);
const reads = (kind: Kind): number[] =>
  kind.kind === "sourceOver" ? [kind.source, kind.destination] : kind.input ? [kind.input] : [];
function fixture() {
  const kinds: Kind[] = [
    { kind: "rasterTree" },
    { kind: "applyOpacity", input: 1 },
    { kind: "rasterTree" },
    { kind: "sourceOver", source: 2, destination: 3 },
    { kind: "applyFilter", input: 4, filter: { kind: "glow" } },
  ];
  const plan = { output: 5, passes: kinds.map((kind) => ({ kind })) };
  const passes = kinds.map((_, i) => ({
    output: i + 1,
    storage: { kind: "surface", slot: i + 1 },
    retireAfter: [],
    deviceRoi: { ...(i === 2 || i === 4 ? output : source) },
  }));
  const bound = {
    passes,
    slotExtents: new Map(
      passes.map((p) => [p.output, { width: p.deviceRoi.width, height: p.deviceRoi.height }]),
    ),
    resourceRois: new Map(passes.map((p) => [p.output, p.deviceRoi])),
    estimatedSurfaceBytes: 1n,
  };
  return { plan, bound };
}
const run = (f: ReturnType<typeof fixture>, kernel = planner) =>
  cropProgramInputs(
    f.plan,
    f.bound,
    kernel,
    (kind) => kind.filter as Kind,
    reads,
    () => [0, 0, 1920, 1080],
  );
test("consumer footprint narrows opacity and composition without mutating admitted packets", () => {
  const f = fixture(),
    result = run(f);
  for (const i of [1, 3]) expect(result.passes[i]!.deviceRoi).toEqual(footprint);
  expect(result.slotExtents.get(1)).toEqual({ width: 4013, height: 3141 });
  expect(result.passes[0]!.deviceRoi).toEqual(source);
  expect(f.bound.passes[0]!.deviceRoi).toEqual(source);
  expect(result.passes[4]!.deviceRoi).toEqual(output);
  expect(result.resourceRois.get(2)).toEqual(footprint);
});
test("another reader retains the producer's complete pixels", () => {
  const f = fixture();
  f.plan.passes.push({ kind: { kind: "applyOpacity", input: 2 } });
  f.bound.passes.push({
    output: 6,
    storage: { kind: "surface", slot: 6 },
    retireAfter: [],
    deviceRoi: { ...source },
  });
  const result = run(f);
  expect(result.passes[0]!.deviceRoi).toEqual(source);
  expect(result.passes[1]!.deviceRoi).toEqual(source);
  expect(result.passes[3]!.deviceRoi).toEqual(footprint);
});
test("an externally consumed program output retains the producer's complete pixels", () => {
  const f = fixture();
  f.plan.output = 2;
  const result = run(f);
  expect(result.passes[1]!.deviceRoi).toEqual(source);
  expect(result.passes[3]!.deviceRoi).toEqual(footprint);
});
test("unknown shader sampling stops backward cropping", () => {
  const f = fixture();
  f.plan.passes[1]!.kind.kind = "applyShader";
  const result = run(f);
  expect(result.passes[0]!.deviceRoi).toEqual(source);
  expect(result.passes[1]!.deviceRoi).toEqual(source);
});
test("a different pyramid sampling phase preserves the original schedule", () => {
  const f = fixture();
  const result = run(f, (...args: unknown[]) => {
    const r = planner();
    if (args[0] !== source.width) r[0]! -= 1;
    return r;
  });
  expect(result).toBe(f.bound);
});
test("device-space transform copies preserve coordinates and authored transforms", () => {
  const f = fixture();
  const transform = [1, 0, 30, 0, 1, -50, 0, 0, 1];
  f.plan.passes[1]!.kind = { kind: "applyTransform", input: 1, transform };
  f.plan.passes[4]!.kind.filter = { kind: "chromaticAberration", offset: [0, 0] };
  const result = run(f);
  expect(result.passes[0]!.deviceRoi).toEqual(source);
  expect(result.passes[1]!.deviceRoi).toEqual({ x: -2, y: -2, width: 1924, height: 1084 });
  expect(f.plan.passes[1]!.kind.transform).toEqual(transform);
});
