import {
  pointFilters,
  samplingFootprint,
  type SampleFilter,
  type SampleRegion,
} from "./sampling-footprint.ts";

type Wire = Record<string, unknown>;
interface PassKind extends Wire {
  kind: string;
  input?: number;
  filter?: SampleFilter;
}
interface LocalPlan {
  output?: number;
  passes: readonly { kind: PassKind }[];
}
interface RoiSchedule {
  readonly passes: readonly {
    readonly output: number;
    readonly deviceRoi: SampleRegion;
    readonly storage: Wire;
  }[];
  readonly slotExtents: ReadonlyMap<number, { width: number; height: number } | null>;
  readonly resourceRois: ReadonlyMap<number, SampleRegion>;
  readonly estimatedSurfaceBytes: bigint;
}
type PyramidPlanner = (
  iw: number,
  ih: number,
  ow: number,
  oh: number,
  x: number,
  y: number,
  filter: string,
) => Float64Array;

/** Narrow only admitted, exclusively consumed device-space producers. Wire packets stay immutable. */
export function cropProgramInputs<T extends RoiSchedule>(
  plan: Wire,
  original: T,
  planner: PyramidPlanner | undefined,
  deviceFilter: (kind: Wire) => Wire,
  reads: (kind: Wire) => readonly number[],
  lensFrame: (kind: Wire) => readonly number[],
  rasterCoverage: (kind: Wire) => boolean = () => false,
): T {
  if (!planner) return original;
  const localPasses = plan.passes as LocalPlan["passes"];
  const passes = original.passes.map((pass) => ({ ...pass }));
  const slotExtents = new Map(original.slotExtents),
    resourceRois = new Map(original.resourceRois);
  const readers = new Map<number, number>();
  for (const pass of localPasses)
    for (const input of reads(pass.kind)) {
      readers.set(input, (readers.get(input) ?? 0) + 1);
    }
  // The program output has a reader outside this local plan.
  if (typeof plan.output === "number")
    readers.set(plan.output, (readers.get(plan.output) ?? 0) + 1);
  const producers = new Map(passes.map((pass, index) => [pass.output, index]));
  let changed = false;
  const crop = (id: number, region: SampleRegion): void => {
    const index = producers.get(id);
    if (index === undefined || readers.get(id) !== 1) return;
    const kind = localPasses[index]!.kind;
    const pointFilter = kind.kind === "applyFilter" && pointFilters.has(kind.filter!.kind);
    const sampledFilter =
      kind.kind === "applyFilter" &&
      ["lensDistortion", "chromaticAberration"].includes(kind.filter!.kind);
    // In this scheduled executor applyTransform copies already transformed device-space pixels.
    // The original transform still determines raster geometry, filters and lens coordinates.
    // RasterTree keeps its original coverage boundary. Clip-to-ROI changes CPU and GPU AA.
    const pixelOperation =
      (kind.kind === "rasterTree" && rasterCoverage(kind)) ||
      [
        "sourceOver",
        "applyOpacity",
        "clear",
        "applyClip",
        "applyMask",
        "blend",
        "applyTransform",
      ].includes(kind.kind);
    if (!pointFilter && !sampledFilter && !pixelOperation) return;
    const pass = passes[index]!,
      a = pass.deviceRoi;
    const x = Math.max(a.x, region.x),
      y = Math.max(a.y, region.y);
    const width = Math.max(0, Math.min(a.x + a.width, region.x + region.width) - x);
    const height = Math.max(0, Math.min(a.y + a.height, region.y + region.height) - y);
    if (width < a.width || height < a.height) {
      pass.deviceRoi = { x, y, width, height };
      resourceRois.set(id, pass.deviceRoi);
      changed = true;
    }
    if (!sampledFilter) for (const input of reads(kind)) crop(input, region);
  };
  for (let i = localPasses.length - 1; i >= 0; i--) {
    const consumer = localPasses[i]!.kind;
    if (consumer.kind !== "applyFilter") continue;
    const inputIndex = producers.get(consumer.input!);
    if (inputIndex === undefined) continue;
    const input = passes[inputIndex]!,
      output = passes[i]!,
      a = input.deviceRoi,
      b = output.deviceRoi;
    if (readers.get(input.output) !== 1 || !a.width || !a.height || !b.width || !b.height) continue;
    const filter = deviceFilter(consumer) as SampleFilter;
    if (!["glow", "bloom"].includes(filter.kind)) {
      const region = samplingFootprint(
        filter,
        b,
        filter.kind === "lensDistortion" ? lensFrame(consumer) : undefined,
      );
      if (region) crop(input.output, region);
      continue;
    }
    const recipe = planner(
      a.width,
      a.height,
      b.width,
      b.height,
      a.x - b.x,
      a.y - b.y,
      JSON.stringify(filter),
    );
    if (recipe.length < 15) continue;
    const x = Math.max(a.x, b.x + recipe[0]!),
      y = Math.max(a.y, b.y + recipe[1]!);
    const right = Math.min(a.x + a.width, b.x + recipe[0]! + recipe[2]!);
    const bottom = Math.min(a.y + a.height, b.y + recipe[1]! + recipe[3]!);
    const width = Math.max(0, right - x),
      height = Math.max(0, bottom - y);
    if (width === a.width && height === a.height) continue;
    if (width && height) {
      const again = planner(
        width,
        height,
        b.width,
        b.height,
        x - b.x,
        y - b.y,
        JSON.stringify(filter),
      );
      if (
        again.length !== recipe.length ||
        again.some((value, index) => index !== 4 && index !== 5 && value !== recipe[index])
      )
        continue;
    }
    crop(input.output, { x, y, width, height });
  }
  if (!changed) return original;
  const extents = new Map<number, { width: number; height: number }>();
  for (const pass of passes) {
    if (pass.storage.kind !== "surface") continue;
    const id = pass.storage.slot as number,
      prior = extents.get(id) ?? { width: 0, height: 0 };
    extents.set(id, {
      width: Math.max(prior.width, pass.deviceRoi.width),
      height: Math.max(prior.height, pass.deviceRoi.height),
    });
  }
  for (const [id, extent] of extents) {
    const originalExtent = slotExtents.get(id);
    if (originalExtent)
      slotExtents.set(
        id,
        extent.width && extent.height
          ? {
              width: Math.min(originalExtent.width, extent.width),
              height: Math.min(originalExtent.height, extent.height),
            }
          : null,
      );
  }
  let estimatedSurfaceBytes = 0n;
  for (const extent of slotExtents.values())
    if (extent) estimatedSurfaceBytes += BigInt(extent.width) * BigInt(extent.height) * 8n;
  return { ...original, passes, slotExtents, resourceRois, estimatedSurfaceBytes };
}
