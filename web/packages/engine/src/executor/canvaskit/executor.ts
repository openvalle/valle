import canvasKitPackage from "canvaskit-wasm/package.json";
import type {
  Canvas,
  CanvasKit,
  BlendMode,
  GrDirectContext,
  Font,
  Image,
  ImageFilter,
  Paint,
  Path,
  RuntimeEffect,
  Shader,
  Surface,
  Typeface,
  MallocObj,
} from "canvaskit-wasm";

import {
  BOUND_PROGRAM_SCHEDULES_ABI,
  RENDER_BINDINGS_ABI,
  RENDER_PLAN_ABI,
  decodePackedAbi,
  type PackedValue,
} from "../../abi/packed.ts";
import { sha256Hex } from "../../abi/sha256.ts";
import {
  BATCH_INSTANCE_BYTES,
  applyDrawProgramPatch,
  decodeDrawProgram,
  type DrawProgramWire,
  type LinearColorWire,
  type RectWire,
} from "./draw-program.ts";
import {
  CanvasKitBuiltinRuntime,
  MOTION_GLASS_UNIFORM_FLOATS,
  TRANSITION_KERNELS,
  type BuiltinKernel,
} from "./builtin-runtime.ts";

type Wire = Record<string, any>;
const IDENTITY_MATRIX = [1, 0, 0, 0, 1, 0, 0, 0, 1];

export interface CanvasKitExternalObject {
  /** Exact ResourceKey from the request fulfilled for this handle. */
  readonly key: PackedValue;
  readonly kind: "visual" | "font" | "runtimeShader" | "scene3d";
  readonly image?: Image;
  readonly bytes?: Uint8Array;
}

export interface CanvasKitObjectTable {
  readonly generation: bigint | number;
  readonly objects: ReadonlyMap<number, CanvasKitExternalObject>;
}

export interface CanvasKitExecutionTarget {
  readonly surface: Surface;
  /** GPU context used for working F16 surfaces. Null selects CanvasKit's CPU surface factory. */
  readonly directContext?: GrDirectContext | null;
  /** Same immutable limits admitted by Engine lowering for this frame. */
  readonly maxSurfaceBytes?: bigint;
  readonly maxFrameBytes?: bigint;
}

/** Canonical Rust/Wasm Motion Glass kernel owned by ProductEngine. */
export interface CanvasKitGlassKernel {
  pack_motion_glass_uniforms(
    programJson: string,
    ownerToDevice: Float64Array,
  ): Float32Array;
  pack_motion_glass_foreground_uniforms(
    programJson: string,
    ownerToDevice: Float64Array,
  ): Float32Array;
}

export interface CanvasKitGlyphCoverageProfile {
  readonly rasterizer: "freetype";
  readonly library: "renderer-embedded";
  readonly hinting: "none";
  readonly edging: "antialias";
  readonly subpixelPositioning: false;
}

export const CANVASKIT_GLYPH_COVERAGE_PROFILE: CanvasKitGlyphCoverageProfile = Object.freeze({
  rasterizer: "freetype",
  library: "renderer-embedded",
  hinting: "none",
  edging: "antialias",
  subpixelPositioning: false,
});

export interface CanvasKitExecutionProfile {
  readonly surfaceBackend: "canvaskit-cpu" | "canvaskit-gpu";
  readonly glyphCoverage: CanvasKitGlyphCoverageProfile;
  readonly canvasKitVersion: string;
}

export interface CanvasKitExecutionReport {
  readonly profile: CanvasKitExecutionProfile;
  readonly passes: number;
  readonly programs: number;
  readonly physicalSurfaces: number;
  readonly maximumLiveImages: number;
  readonly surfaceAllocations: number;
  readonly surfaceReuses: number;
  readonly surfaceAllocatedBytes: bigint;
  readonly surfaceResidentBytes: bigint;
  readonly surfacePeakBytes: bigint;
  readonly surfacePoolEvictions: number;
  readonly programCacheHits: number;
  readonly programCacheMisses: number;
  readonly fontCacheHits: number;
  readonly fontCacheMisses: number;
  readonly shaderCacheHits: number;
  readonly shaderCacheMisses: number;
  readonly packetAdmissionMs: number;
  readonly programAdmissionMs: number;
  readonly scheduleAdmissionMs: number;
  readonly renderMs: number;
  readonly programLocalPasses: number;
  readonly directRasterPrograms: number;
  readonly programGroups: number;
  readonly programTransformGroups: number;
  readonly programClipGroups: number;
  readonly programOpacityGroups: number;
  readonly programFilterGroups: number;
  readonly programMaskGroups: number;
  readonly programShaderGroups: number;
  readonly programBackdropGroups: number;
  readonly programBlendGroups: number;
  readonly committed: true;
}

export class CanvasKitCompositorError extends Error {
  constructor(readonly code: string, message: string) {
    super(`[${code}] ${message}`);
    this.name = "CanvasKitCompositorError";
  }
}

interface AdmittedProgram {
  readonly plan: Wire;
  readonly draw: DrawProgramWire;
  readonly directRaster: DirectRasterPlan | null;
  readonly textures: Map<string, CanvasKitExternalObject>;
  readonly fonts: Map<string, Typeface>;
  readonly shaders: Map<string, RuntimeEffect>;
  readonly scenes: Map<string, CanvasKitExternalObject>;
}

interface DirectRasterPlan {
  readonly roots: readonly number[];
  readonly layerBounds: ReadonlyMap<number, RectWire>;
  readonly backdrops: ReadonlyMap<number, DirectRasterBackdrop>;
}

interface DirectRasterBackdrop {
  readonly destination: number | null;
  readonly hasLocalInputs: boolean;
}

export interface ImageValue {
  readonly image: Image | null;
  readonly owned: boolean;
  readonly roi: DeviceRoi;
}

interface BoundProgramRuntimeSchedule {
  readonly resourceRois: Map<number, DeviceRoi>;
  readonly slotExtents: Map<number, { width: number; height: number } | null>;
  readonly slotByResource: Map<number, number>;
  readonly estimatedSurfaceBytes: bigint;
}

export interface DeviceRoi {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

interface ExecutorCacheCounters {
  programHits: number;
  programMisses: number;
  fontHits: number;
  fontMisses: number;
  shaderHits: number;
  shaderMisses: number;
}

function emptyExecutorCacheCounters(): ExecutorCacheCounters {
  return {
    programHits: 0,
    programMisses: 0,
    fontHits: 0,
    fontMisses: 0,
    shaderHits: 0,
    shaderMisses: 0,
  };
}

function cacheCounterReport(
  before: ExecutorCacheCounters,
  after: ExecutorCacheCounters,
): Pick<CanvasKitExecutionReport,
  "programCacheHits" | "programCacheMisses" |
  "fontCacheHits" | "fontCacheMisses" |
  "shaderCacheHits" | "shaderCacheMisses"> {
  return {
    programCacheHits: after.programHits - before.programHits,
    programCacheMisses: after.programMisses - before.programMisses,
    fontCacheHits: after.fontHits - before.fontHits,
    fontCacheMisses: after.fontMisses - before.fontMisses,
    shaderCacheHits: after.shaderHits - before.shaderHits,
    shaderCacheMisses: after.shaderMisses - before.shaderMisses,
  };
}

function touch<K, V>(values: Map<K, V>, key: K, value: V): void {
  values.delete(key);
  values.set(key, value);
}

function trimPlainCache<K, V>(values: Map<K, V>, maximum: number): void {
  while (values.size > maximum) {
    const oldest = values.keys().next();
    if (oldest.done) return;
    values.delete(oldest.value);
  }
}

function trimDeleteableCache<K, V extends { delete(): void }>(values: Map<K, V>, maximum: number): void {
  while (values.size > maximum) {
    const oldest = values.entries().next();
    if (oldest.done) return;
    oldest.value[1].delete();
    values.delete(oldest.value[0]);
  }
}

/** Long-lived Web executor. Built-in kernels and F16 working surfaces are reused across frames. */
export class CanvasKitExecutor {
  readonly builtins: CanvasKitBuiltinRuntime;
  private workingArena: SurfaceArena | null = null;
  private readonly drawPrograms = new Map<string, Promise<DrawProgramWire>>();
  private readonly baselinePrograms = new Map<string, Promise<Uint8Array>>();
  private readonly baselineFrames = new Map<string, Promise<Uint8Array>>();
  private readonly planTemplates = new Map<
    string,
    { packet: Uint8Array; admitted: Promise<Wire> }
  >();
  private readonly typefaces = new Map<string, Typeface>();
  private readonly runtimeShaders = new Map<string, RuntimeEffect>();
  private cacheCounters = emptyExecutorCacheCounters();

  constructor(
    readonly CanvasKit: CanvasKit,
    readonly glassKernel: CanvasKitGlassKernel | null = null,
  ) {
    this.builtins = new CanvasKitBuiltinRuntime(CanvasKit);
  }

  async execute(
    planPacket: ArrayBuffer | ArrayBufferView,
    bindingPacket: ArrayBuffer | ArrayBufferView,
    schedulePacket: ArrayBuffer | ArrayBufferView,
    objectTable: CanvasKitObjectTable,
    target: CanvasKitExecutionTarget,
  ): Promise<CanvasKitExecutionReport> {
    return executeCanvasKitFrame(this, planPacket, bindingPacket, schedulePacket, objectTable, target);
  }

  arena(
    context: GrDirectContext | null,
    width: number,
    height: number,
    maxSurfaceBytes: bigint,
    maxFrameBytes: bigint,
  ): SurfaceArena {
    if (!this.workingArena?.matches(context, width, height, maxSurfaceBytes, maxFrameBytes)) {
      this.workingArena?.delete();
      this.workingArena = new SurfaceArena(
        this.CanvasKit,
        context,
        width,
        height,
        maxSurfaceBytes,
        maxFrameBytes,
      );
    }
    this.workingArena.beginFrame();
    return this.workingArena;
  }

  dispose(): void {
    this.workingArena?.delete();
    this.workingArena = null;
    this.builtins.dispose();
    for (const face of this.typefaces.values()) face.delete();
    for (const effect of this.runtimeShaders.values()) effect.delete();
    this.drawPrograms.clear();
    this.baselinePrograms.clear();
    this.baselineFrames.clear();
    this.planTemplates.clear();
    this.typefaces.clear();
    this.runtimeShaders.clear();
  }

  async drawProgram(key: string, packed: Uint8Array): Promise<DrawProgramWire> {
    const cached = this.drawPrograms.get(key);
    if (cached) {
      this.cacheCounters.programHits += 1;
      touch(this.drawPrograms, key, cached);
      return cached;
    }
    this.cacheCounters.programMisses += 1;
    const pending = decodeDrawProgram(packed);
    this.drawPrograms.set(key, pending);
    try { return await pending; }
    catch (error) { this.drawPrograms.delete(key); throw error; }
  }

  async baselineProgram(key: string, encoded: unknown): Promise<Uint8Array> {
    const cached = this.baselinePrograms.get(key);
    if (cached) {
      touch(this.baselinePrograms, key, cached);
      return cached;
    }
    const packed = decodeBase64Bytes(encoded, "program.baselinePacked");
    const pending = Promise.all([
      packedHash(packed).then((actual) => {
        if (actual !== key) fail("program_contract", "baseline content hash mismatch");
      }),
      // Decode the template-owned baseline once at the Web trust boundary. This also primes the
      // exact DrawProgram cache when the baseline is itself the requested frame.
      this.drawProgram(key, packed),
    ]).then(() => packed);
    this.baselinePrograms.set(key, pending);
    try { return await pending; }
    catch (error) { this.baselinePrograms.delete(key); throw error; }
  }

  async baselineFrame(key: string, encoded: unknown): Promise<Uint8Array> {
    const cached = this.baselineFrames.get(key);
    if (cached) {
      touch(this.baselineFrames, key, cached);
      return cached;
    }
    const packed = decodeBase64Bytes(encoded, "program.baselineFramePacked");
    const pending = packedHash(packed).then((actual) => {
      if (actual !== key) fail("program_contract", "baseline frame hash mismatch");
      return packed;
    });
    this.baselineFrames.set(key, pending);
    try { return await pending; }
    catch (error) { this.baselineFrames.delete(key); throw error; }
  }

  async planTemplate(
    key: string,
    packetInput: ArrayBuffer | ArrayBufferView,
  ): Promise<Wire> {
    const packet = exactPacketBytes(packetInput);
    const cached = this.planTemplates.get(key);
    if (cached
      && cached.packet.buffer === packet.buffer
      && cached.packet.byteOffset === packet.byteOffset
      && cached.packet.byteLength === packet.byteLength) {
      touch(this.planTemplates, key, cached);
      return cached.admitted;
    }
    const admitted = Promise.all([
      packedHash(packet).then((actual) => {
        if (actual !== key) fail("template_hash", "bindings do not name the supplied plan packet");
      }),
      decodePackedAbi(packet, RENDER_PLAN_ABI),
    ]).then(([, value]) => record(value, "RenderPlanTemplate"));
    const entry = { packet, admitted };
    this.planTemplates.set(key, entry);
    try { return await admitted; }
    catch (error) { this.planTemplates.delete(key); throw error; }
  }

  typeface(key: string, faceIndex: number, bytes: Uint8Array): Typeface {
    // CanvasKit 0.41 does not expose SkFontArguments::CollectionIndex: this API always decodes
    // face zero. Engine admission rejects non-zero collection faces for the Native/Web common
    // profile; keep this trust boundary fail-closed for foreign or stale render-plan packets too.
    if (faceIndex !== 0) {
      fail("font_admission", `font '${key}' requests unsupported collection face ${faceIndex}`);
    }
    const cached = this.typefaces.get(key);
    if (cached) {
      this.cacheCounters.fontHits += 1;
      touch(this.typefaces, key, cached);
      return cached;
    }
    this.cacheCounters.fontMisses += 1;
    const face = this.CanvasKit.Typeface.MakeFreeTypeFaceFromData(bytes.slice().buffer);
    if (!face) fail("font_admission", `font '${key}' cannot be decoded`);
    this.typefaces.set(key, face);
    return face;
  }

  runtimeShader(key: string, bytes: Uint8Array): RuntimeEffect {
    const cached = this.runtimeShaders.get(key);
    if (cached) {
      this.cacheCounters.shaderHits += 1;
      touch(this.runtimeShaders, key, cached);
      return cached;
    }
    this.cacheCounters.shaderMisses += 1;
    let detail = "";
    const effect = this.CanvasKit.RuntimeEffect.Make(
      new TextDecoder("utf-8", { fatal: true }).decode(bytes),
      (message) => { detail = message; },
    );
    if (!effect) fail("shader_admission", `shader '${key}' failed: ${detail}`);
    this.runtimeShaders.set(key, effect);
    return effect;
  }

  cacheSnapshot(): ExecutorCacheCounters {
    return { ...this.cacheCounters };
  }

  trimCaches(): void {
    trimDeleteableCache(this.typefaces, 256);
    trimDeleteableCache(this.runtimeShaders, 128);
    trimPlainCache(this.drawPrograms, 256);
    trimPlainCache(this.baselinePrograms, 64);
    trimPlainCache(this.baselineFrames, 64);
    trimPlainCache(this.planTemplates, 64);
  }
}

/**
 * Sealed Web Product Compositor executor.
 *
 * Both binary packets and every external object are admitted before a working surface is
 * allocated. Rendering happens in private slot-pooled surfaces and the caller's target receives
 * one final image only after the complete plan succeeds.
 */
export async function executeCanvasKitCompositor(
  CanvasKit: CanvasKit,
  planPacket: ArrayBuffer | ArrayBufferView,
  bindingPacket: ArrayBuffer | ArrayBufferView,
  schedulePacket: ArrayBuffer | ArrayBufferView,
  objectTable: CanvasKitObjectTable,
  target: CanvasKitExecutionTarget,
): Promise<CanvasKitExecutionReport> {
  const executor = new CanvasKitExecutor(CanvasKit);
  try {
    return await executor.execute(planPacket, bindingPacket, schedulePacket, objectTable, target);
  } finally {
    executor.dispose();
  }
}

async function executeCanvasKitFrame(
  executor: CanvasKitExecutor,
  planPacket: ArrayBuffer | ArrayBufferView,
  bindingPacket: ArrayBuffer | ArrayBufferView,
  schedulePacket: ArrayBuffer | ArrayBufferView,
  objectTable: CanvasKitObjectTable,
  target: CanvasKitExecutionTarget,
): Promise<CanvasKitExecutionReport> {
  const { CanvasKit, builtins } = executor;
  const packetAdmissionStarted = performance.now();
  executor.trimCaches();
  const cacheBefore = executor.cacheSnapshot();
  const [bindingsValue, scheduleValue, bindingHash] = await Promise.all([
    decodePackedAbi(bindingPacket, RENDER_BINDINGS_ABI),
    decodePackedAbi(schedulePacket, BOUND_PROGRAM_SCHEDULES_ABI),
    packedHash(bindingPacket),
  ]);
  const bindings = record(bindingsValue, "RenderBindings");
  const schedule = record(scheduleValue, "BoundProgramSchedules");
  const templateHash = digestString(bindings.templateHash, "bindings.templateHash");
  const plan = await executor.planTemplate(templateHash, planPacket);
  validatePacketTriple(plan, bindings, schedule, templateHash, bindingHash);
  const generation = exactPositiveInteger(bindings.externalGeneration, "bindings.externalGeneration");
  if (generation !== BigInt(objectTable.generation)) {
    fail("stale_generation", `binding generation ${generation} does not match object table ${objectTable.generation}`);
  }

  const externalIds = array(bindings.externalIds, "bindings.externalIds").map((value, index) =>
    positiveId(value, `bindings.externalIds[${index}]`));
  const externalSlots = array(record(plan.bindingLayout, "plan.bindingLayout").externalSlots, "externalSlots");
  if (externalIds.length !== externalSlots.length) fail("binding_layout", "external slot count mismatch");
  const externalBySlot = new Map<number, CanvasKitExternalObject>();
  for (let index = 0; index < externalIds.length; index += 1) {
    const slot = record(externalSlots[index], `externalSlots[${index}]`);
    const slotId = positiveId(slot.id, `externalSlots[${index}].id`);
    if (slotId !== index + 1) fail("binding_layout", "external slots are not canonical");
    const object = objectTable.objects.get(externalIds[index]!);
    if (!object) fail("missing_external", `external handle ${externalIds[index]} is absent`);
    if (!deepEqual(object.key, slot.key)) fail("external_contract", `external slot ${slotId} key mismatch`);
    externalBySlot.set(slotId, object);
  }

  const programs = new Map<number, AdmittedProgram>();
  const packetAdmissionMs = performance.now() - packetAdmissionStarted;
  const programAdmissionStarted = performance.now();
  const programLayouts = array(plan.programs, "plan.programs");
  const framePrograms = array(bindings.programs, "bindings.programs");
  if (framePrograms.length !== programLayouts.length) {
    fail("program_layout", "frame program count does not match the template layout");
  }
  const programContracts: Wire[] = [];
  for (let index = 0; index < programLayouts.length; index += 1) {
    const layout = record(programLayouts[index], `plan.programs[${index}]`);
    const frameProgram = record(framePrograms[index], `bindings.programs[${index}]`);
    const contract = admitFrameProgramLayout(layout, frameProgram, index);
    const admitted = await admitProgram(executor, contract, externalBySlot);
    programContracts.push(admitted.plan);
    const id = positiveId(admitted.plan.id, `bindings.programs[${index}].id`);
    if (programs.has(id)) fail("duplicate_program", `program ${id} is duplicated`);
    programs.set(id, admitted);
  }

  const programAdmissionMs = performance.now() - programAdmissionStarted;
  const programDiagnostics = inspectProgramDiagnostics(programs);
  const scheduleAdmissionStarted = performance.now();
  admitCanvasKitOutput(plan);
  const requiredKernels = requiredBuiltinKernels(plan, programContracts);
  requiredKernels.add("normalizeInput");
  builtins.admit(requiredKernels);

  const extent = renderExtent(plan);
  preflightTarget(target.surface, extent);
  const dynamic = dynamicBindings(bindings);
  preflightEffectDomains(plan, dynamic, extent);
  const boundSchedule = admitBoundOuterSchedule(plan, programContracts, schedule, dynamic, extent);
  const rootBytes = BigInt(extent.width) * BigInt(extent.height) * 8n;
  const maxSurfaceBytes = target.maxSurfaceBytes ?? maxBigInt(rootBytes, 64n * 1024n * 1024n);
  const maxFrameBytes = target.maxFrameBytes ?? maxBigInt(maxSurfaceBytes * 16n, 512n * 1024n * 1024n);
  const arena = executor.arena(
    target.directContext ?? null,
    extent.width,
    extent.height,
    maxSurfaceBytes,
    maxFrameBytes,
  );
  const resources = new Map<number, ImageValue>();
  const surfaceSlotByResource = boundSchedule.slotByResource;
  const externalResourceById = new Map<number, CanvasKitExternalObject>();
  for (const raw of array(plan.resources, "plan.resources")) {
    const resource = record(raw, "plan resource");
    const kind = record(resource.kind, "plan resource kind");
    if (kind.kind === "external") {
      externalResourceById.set(positiveId(resource.id, "resource.id"), required(externalBySlot, positiveId(kind.slot, "resource.slot"), "external slot"));
    }
  }
  const scheduleAdmissionMs = performance.now() - scheduleAdmissionStarted;

  let maximumLiveImages = 0;
  let output: Image | null = null;
  const renderStarted = performance.now();
  try {
    for (const [index, rawPass] of array(plan.passes, "plan.passes").entries()) {
      const pass = record(rawPass, `passes[${index}]`);
      if (positiveId(pass.id, `passes[${index}].id`) !== index + 1) fail("pass_order", "pass ids are not canonical");
      const kind = record(pass.kind, `passes[${index}].kind`);
      const outputId = passOutput(kind);
      if (kind.kind === "bindBackdropView" || kind.kind === "aliasResource") {
        resources.set(outputId, borrowedValue(resourceValue(resources, kind.input)));
        retireOuterResources(plan, index + 1, resources, outputId);
        maximumLiveImages = Math.max(maximumLiveImages, liveImages(resources));
        continue;
      }
      const outputRoi = required(boundSchedule.resourceRois, outputId, "bound output ROI");
      if (outputRoi.width === 0 || outputRoi.height === 0) {
        resources.set(outputId, transparentValue());
        retireOuterResources(plan, index + 1, resources, outputId);
        continue;
      }
      const slot = surfaceSlotByResource.get(outputId);
      const slotExtent = slot === undefined
        ? extent
        : required(boundSchedule.slotExtents, slot, "bound surface slot extent");
      if (slotExtent === null) fail("bound_schedule", `visible output ${outputId} has no physical slot extent`);
      const surface = slot === undefined
        ? arena.output()
        : arena.slot(slot, slotExtent.width, slotExtent.height);
      const canvas = surface.getCanvas();
      canvas.clear(CanvasKit.TRANSPARENT);
      canvas.save();
      canvas.translate(-outputRoi.x, -outputRoi.y);

      try { switch (kind.kind) {
        case "clearRegion":
          canvas.clear(color4(kind.workingLinearRec2020Premul));
          break;
        case "importRegion":
          renderImport(CanvasKit, builtins, canvas, externalObject(externalResourceById, kind.external), kind, dynamic, externalResourceById);
          break;
        case "rasterProgram": {
          const program = required(programs, positiveId(kind.program, "RasterProgram.program"), "program");
          const destinations = array(kind.destinationInputs, "destinationInputs").map((id) =>
            borrowedValue(resourceValue(resources, id)));
          const result = executeProgram(
            CanvasKit,
            builtins,
            arena,
            executor.glassKernel,
            program,
            required(boundSchedule.programs, positiveId(kind.program, "RasterProgram.program"), "bound program schedule"),
            destinations,
            dynamicTransform(dynamic, kind.transform),
            extent,
          );
          drawImageValue(CanvasKit, canvas, result, 1, "src");
          if (result.owned) result.image?.delete();
          break;
        }
        case "rasterCaption": {
          const program = required(programs, positiveId(kind.program, "RasterCaption.program"), "program");
          const result = executeProgram(
            CanvasKit,
            builtins,
            arena,
            executor.glassKernel,
            program,
            required(boundSchedule.programs, positiveId(kind.program, "RasterCaption.program"), "bound program schedule"),
            [],
            dynamicTransform(dynamic, kind.transform),
            extent,
          );
          drawImageValue(CanvasKit, canvas, resourceValue(resources, kind.destination), 1, "src");
          drawImageValue(CanvasKit, canvas, result, dynamicScalar(dynamic, kind.opacity), "srcOver");
          if (result.owned) result.image?.delete();
          break;
        }
        case "resolveRegion": {
          const bounds = dynamicBounds(dynamic, kind.sampleBounds);
          canvas.save();
          canvas.clipRect(deviceRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, false);
          drawImageValue(CanvasKit, canvas, resourceValue(resources, kind.input), 1, "src");
          canvas.restore();
          break;
        }
        case "dispatchKernel":
          {
            const rootResources = materializeRootResources(CanvasKit, arena, resources, extent);
            try {
              executeKernel(CanvasKit, builtins, arena, canvas, record(kind.invocation, "kernel invocation"), rootResources, dynamic, extent);
            } finally {
              disposeMaterializedResources(rootResources, resources);
            }
          }
          break;
        case "compositeRegion": {
          const mode = record(kind.mode, "composite mode");
          const source = resourceValue(resources, kind.layer);
          const backdrop = resourceValue(resources, kind.backdrop);
          if (mode.kind === "blend" && String(mode.mode) !== "normal") {
            const rootSource = materializeRootValue(CanvasKit, arena, source, extent);
            const rootBackdrop = materializeRootValue(CanvasKit, arena, backdrop, extent);
            drawCreativeBlend(
              CanvasKit,
              builtins,
              canvas,
              rootSource.image,
              rootBackdrop.image,
              String(mode.mode),
              dynamicScalar(dynamic, kind.opacity),
              true,
            );
            if (rootSource.owned) rootSource.image?.delete();
            if (rootBackdrop.owned) rootBackdrop.image?.delete();
          } else {
            drawImageValue(CanvasKit, canvas, backdrop, 1, "src");
            drawImageValue(CanvasKit, canvas, source, dynamicScalar(dynamic, kind.opacity), "srcOver");
          }
          break;
        }
        case "copyConvert": {
          const input = materializeRootValue(CanvasKit, arena, resourceValue(resources, kind.input), extent);
          try {
            drawOutputTransform(
              CanvasKit,
              builtins,
              canvas,
              input.image,
              record(kind.operation, "copy operation"),
            );
          } finally {
            if (input.owned) input.image?.delete();
          }
          break;
        }
        default:
          fail("unsupported_pass", `execution pass '${String(kind.kind)}' is not closed in the CanvasKit executor`);
      }} finally { canvas.restore(); }

      surface.flush();
      const snapshot = surface.makeImageSnapshot();
      resources.set(outputId, { image: snapshot, owned: true, roi: outputRoi });
      retireOuterResources(plan, index + 1, resources, outputId);
      maximumLiveImages = Math.max(maximumLiveImages, liveImages(resources));
    }

    output = image(resources, positiveId(plan.output, "plan.output"));
    const targetCanvas = target.surface.getCanvas();
    targetCanvas.clear(CanvasKit.TRANSPARENT);
    drawImage(CanvasKit, targetCanvas, output, 1, "src");
    target.surface.flush();
    arena.prepareReport();
    return {
      profile: {
        surfaceBackend: target.directContext ? "canvaskit-gpu" : "canvaskit-cpu",
        glyphCoverage: CANVASKIT_GLYPH_COVERAGE_PROFILE,
        canvasKitVersion: canvasKitPackage.version,
      },
      passes: array(plan.passes, "plan.passes").length,
      programs: programs.size,
      physicalSurfaces: arena.count,
      maximumLiveImages,
      surfaceAllocations: arena.frameAllocations,
      surfaceReuses: arena.frameReuses,
      surfaceAllocatedBytes: arena.frameAllocatedBytes,
      surfaceResidentBytes: arena.residentBytes,
      surfacePeakBytes: arena.framePeakBytes,
      surfacePoolEvictions: arena.frameEvictions,
      ...cacheCounterReport(cacheBefore, executor.cacheSnapshot()),
      packetAdmissionMs,
      programAdmissionMs,
      scheduleAdmissionMs,
      renderMs: performance.now() - renderStarted,
      ...programDiagnostics,
      committed: true,
    };
  } finally {
    disposeValues(resources, output);
    disposePrograms(programs);
    arena.endFrame();
    executor.trimCaches();
  }
}

function inspectProgramDiagnostics(programs: ReadonlyMap<number, AdmittedProgram>) {
  const report = {
    programLocalPasses: 0,
    directRasterPrograms: 0,
    programGroups: 0,
    programTransformGroups: 0,
    programClipGroups: 0,
    programOpacityGroups: 0,
    programFilterGroups: 0,
    programMaskGroups: 0,
    programShaderGroups: 0,
    programBackdropGroups: 0,
    programBlendGroups: 0,
  };
  for (const program of programs.values()) {
    report.programLocalPasses += array(record(program.plan.localPlan, "program.localPlan").passes, "program passes").length;
    report.directRasterPrograms += Number(program.directRaster !== null);
    for (const rawNode of program.draw.nodes) {
      const node = record(rawNode, "DrawProgram node");
      if (node.kind !== "group") continue;
      const group = record(node.value, "DrawProgram group");
      report.programGroups += 1;
      report.programTransformGroups += Number(!deepEqual(group.transform, IDENTITY_MATRIX));
      report.programClipGroups += Number(group.clip != null);
      report.programOpacityGroups += Number(finiteNumber(group.opacity, "group opacity") !== 1);
      report.programFilterGroups += Number(array(group.filters, "group filters").length !== 0);
      report.programMaskGroups += Number(group.mask != null);
      report.programShaderGroups += Number(group.shader != null);
      report.programBackdropGroups += Number(group.backdrop != null);
      report.programBlendGroups += Number(String(group.internalBlend) !== "normal");
    }
  }
  return report;
}

async function admitProgram(
  executor: CanvasKitExecutor,
  plan: Wire,
  objects: Map<number, CanvasKitExternalObject>,
): Promise<AdmittedProgram> {
  const { CanvasKit } = executor;
  const baselineFrameHash = digestString(plan.baselineFrameHash, "program.baselineFrameHash");
  const baselineHash = digestString(plan.baselineContentHash, "program.baselineContentHash");
  const [baselineFrame, baseline] = await Promise.all([
    executor.baselineFrame(baselineFrameHash, plan.baselineFramePacked),
    executor.baselineProgram(baselineHash, plan.baselinePacked),
  ]);
  const framePatch = decodeBase64Bytes(plan.framePatch, "program.framePatch");
  const framePacked = applyDrawProgramPatch(baselineFrame, framePatch);
  const frameHash = digestString(plan.frameHash, "program.frameHash");
  const patch = decodeBase64Bytes(plan.patch, "program.patch");
  const packed = applyDrawProgramPatch(baseline, patch);
  const contentHash = digestString(plan.contentHash, "program.contentHash");
  const [actualFrameHash, actualContentHash, draw] = await Promise.all([
    packedHash(framePacked),
    packedHash(packed),
    executor.drawProgram(contentHash, packed),
  ]);
  if (actualFrameHash !== frameHash) {
    fail("program_contract", `program ${String(plan.id)} reconstructed frame hash mismatch`);
  }
  if (actualContentHash !== contentHash) {
    fail("program_contract", `program ${String(plan.id)} reconstructed content hash mismatch`);
  }
  let frameValue: unknown;
  try {
    frameValue = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(framePacked));
  } catch (error) {
    fail("program_contract", `program ${String(plan.id)} frame metadata is invalid: ${String(error)}`);
  }
  const exactPlan = { ...plan, ...record(frameValue, "program frame metadata") };
  if (!deepEqual(draw.viewport, exactPlan.viewport) || !deepEqual(draw.requirements, exactPlan.requirements)) {
    fail("program_contract", `program ${String(exactPlan.id)} packed contract mismatch`);
  }
  const resources = record(exactPlan.resources, "program.resources");
  const requirements = record(exactPlan.requirements, "program.requirements");
  const textures = admitProgramTextures(
    requirements.externalTextures,
    resources.textures,
    objects,
  );
  const fonts = new Map<string, Typeface>();
  for (const raw of array(resources.fonts, "program fonts")) {
    const binding = record(raw, "font binding");
    const object = required(objects, positiveId(binding.slot, "font slot"), "font slot");
    if (object.kind !== "font" || !object.bytes) fail("font_contract", "font slot has no bytes");
    const faceIndex = positiveIdOrZero(binding.faceIndex, "faceIndex");
    // PreparedFrame uses the public ContentDigest spelling while DrawProgram's sealed internal
    // font ABI stores the same digest as bare lowercase hex. Normalize only at this typed bridge.
    const faceHash = digestString(binding.faceHash, "font binding faceHash").slice("sha256:".length);
    const key = fontKey(faceHash, faceIndex);
    fonts.set(key, executor.typeface(key, faceIndex, object.bytes));
  }
  const shaders = new Map<string, RuntimeEffect>();
  for (const raw of array(resources.runtimeShaders, "program shaders")) {
    const binding = record(raw, "shader binding");
    const object = required(objects, positiveId(binding.slot, "shader slot"), "shader slot");
    if (object.kind !== "runtimeShader" || !object.bytes) fail("shader_contract", "shader slot has no source bytes");
    const key = `${String(binding.key)}:${JSON.stringify(object.key, bigintJson)}`;
    shaders.set(String(binding.key), executor.runtimeShader(key, object.bytes));
  }
  const scenes = new Map<string, CanvasKitExternalObject>();
  for (const raw of array(resources.scenes, "program scenes")) {
    const binding = record(raw, "scene binding");
    const object = required(objects, positiveId(binding.slot, "scene slot"), "scene slot");
    if (object.kind !== "scene3d" || !object.image) fail("scene_contract", "Scene3D slot has no raster image");
    scenes.set(String(binding.key), object);
  }
  return {
    plan: exactPlan,
    draw,
    directRaster: directRasterPlan(draw, record(exactPlan.localPlan, "program.localPlan")),
    textures,
    fonts,
    shaders,
    scenes,
  };
}

function admitFrameProgramLayout(layout: Wire, program: Wire, index: number): Wire {
  const label = `program ${index + 1}`;
  const layoutId = positiveId(layout.id, `plan.programs[${index}].id`);
  if (layoutId !== index + 1
    || positiveId(program.id, `bindings.programs[${index}].id`) !== layoutId
  ) {
    fail("program_layout", `${label} does not match its reusable template slot`);
  }
  digestString(layout.structureHash, `plan.programs[${index}].structureHash`);
  digestString(layout.baselineContentHash, `plan.programs[${index}].baselineContentHash`);
  digestString(layout.baselineFrameHash, `plan.programs[${index}].baselineFrameHash`);
  digestString(program.contentHash, `bindings.programs[${index}].contentHash`);
  digestString(program.frameHash, `bindings.programs[${index}].frameHash`);
  return { ...layout, ...program };
}

function executeProgram(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  arena: SurfaceArena,
  glassKernel: CanvasKitGlassKernel | null,
  admitted: AdmittedProgram,
  bound: BoundProgramRuntimeSchedule,
  destinations: ImageValue[],
  device: number[],
  extent: { width: number; height: number },
): ImageValue {
  const localPlan = record(admitted.plan.localPlan, "program.localPlan");
  if (admitted.directRaster) {
    return executeDirectRasterProgram(
      CanvasKit,
      builtins,
      arena,
      admitted,
      bound,
      destinations,
      device,
      localPlan,
    );
  }
  const schedule = record(admitted.plan.localSchedule, "program.localSchedule");
  const storageByResource = new Map<number, Wire>();
  for (const raw of array(schedule.resources, "localSchedule.resources")) {
    const value = record(raw, "program resource storage");
    storageByResource.set(positiveId(value.resource, "program resource storage id"), record(value.storage, "program storage"));
  }
  const values = new Map<number, ImageValue>();
  const localSlotImages = new Map<number, Image>();
  const retiredImages = new Set<Image>();
  let result = transparentValue();
  try {
    for (const rawPass of array(localPlan.passes, "localPlan.passes")) {
      const pass = record(rawPass, "program pass");
      const kind = record(pass.kind, "program pass kind");
      const output = passOutput(kind);
      const storage = required(storageByResource, output, "program resource storage");
      if (storage.kind === "transparent") {
        values.set(output, transparentValue());
        continue;
      }
      if (storage.kind === "destination") {
        const destination = destinations[positiveId(storage.destination, "destination") - 1];
        if (!destination) fail("program_schedule", `destination ${String(storage.destination)} is absent`);
        values.set(output, borrowedValue(destination));
        continue;
      }
      if (storage.kind === "alias") {
        values.set(output, borrowedValue(required(values, positiveId(storage.source, "program alias source"), "program alias source")));
        continue;
      }

      const outputRoi = required(bound.resourceRois, output, "bound program output ROI");
      if (outputRoi.width === 0 || outputRoi.height === 0) {
        fail("program_schedule", `materialized program resource ${output} has an empty bound ROI`);
      }
      const slot = storage.kind === "surface"
        ? required(bound.slotByResource, output, "bound program surface slot")
        : 0;
      let surface: Surface;
      if (slot === 0) {
        surface = arena.transient(outputRoi.width, outputRoi.height);
      } else {
        const prior = localSlotImages.get(slot);
        if (prior) {
          retiredImages.add(prior);
          prior.delete();
        }
        const slotExtent = required(bound.slotExtents, slot, "bound program slot extent");
        if (!slotExtent) fail("program_schedule", `visible program slot ${slot} has no extent`);
        surface = arena.programSlot(
          positiveId(admitted.plan.id, "program id"),
          slot,
          slotExtent.width,
          slotExtent.height,
        );
      }
      const snapshot = (() => {
        try {
          const canvas = surface.getCanvas();
          canvas.clear(CanvasKit.TRANSPARENT);
          canvas.save();
          canvas.translate(-outputRoi.x, -outputRoi.y);
          try {
          executeProgramPass(
            CanvasKit,
            builtins,
            glassKernel,
            canvas,
            admitted,
            kind,
            values,
            destinations,
            device,
            localPlan,
            outputRoi,
          );
          } finally {
            canvas.restore();
          }
          surface.flush();
          return surface.makeImageSnapshot();
        } finally {
          if (slot === 0) arena.releaseTransient(surface);
        }
      })();
      if (slot !== 0) localSlotImages.set(slot, snapshot);
      const value = { image: snapshot, owned: true, roi: outputRoi };
      values.set(output, value);
      if (storage.kind === "output") result = value;
    }
    if (result.image === null) result = values.get(positiveId(localPlan.output, "localPlan.output")) ?? result;
    // `result` is deliberately excluded from the cleanup below and ownership crosses this
    // function boundary. The outer pass must delete it after copying it into its own surface;
    // reporting it as borrowed leaks one snapshot per DrawProgram execution.
    return { image: result.image, owned: true, roi: result.roi };
  } finally {
    const liveSlotImages = new Set(localSlotImages.values());
    for (const [id, value] of values) {
      if (value.owned
        && value.image
        && value.image !== result.image
        && !retiredImages.has(value.image)
        && !liveSlotImages.has(value.image)) {
        value.image.delete();
      }
      values.delete(id);
    }
    for (const image of liveSlotImages) {
      if (image !== result.image && !retiredImages.has(image)) image.delete();
    }
  }
}

function directRasterPlan(draw: DrawProgramWire, localPlan: Wire): DirectRasterPlan | null {
  const destinationUses = array(draw.requirements.destinationUses, "DrawProgram destination uses");
  const resources = array(localPlan.resources, "direct raster resources").map((raw) =>
    record(raw, "direct raster resource"));
  const layerBounds = new Map<number, RectWire>();
  const backdrops = new Map<number, DirectRasterBackdrop>();
  for (const rawPass of array(localPlan.passes, "direct raster passes")) {
    const kind = record(record(rawPass, "direct raster pass").kind, "direct raster pass kind");
    if (kind.kind === "readDestination") {
      if (String(kind.operation) !== "backdrop") return null;
      const node = positiveIdOrZero(kind.node, "direct raster backdrop node");
      if (backdrops.has(node)) return null;
      const localInputs = array(kind.localInputs, "direct raster destination local inputs");
      backdrops.set(node, {
        destination: kind.external == null
          ? null
          : positiveId(kind.external, "direct raster backdrop destination") - 1,
        hasLocalInputs: localInputs.length !== 0,
      });
      continue;
    }
    if (kind.kind !== "applyOpacity" && kind.kind !== "applyFilter") continue;
    const node = positiveIdOrZero(kind.node, "direct raster layer node");
    const resourceId = positiveId(
      kind.kind === "applyOpacity" ? kind.input : kind.output,
      "direct raster layer resource",
    );
    const resource = record(requiredIndex(resources, resourceId - 1, "direct raster layer resource"), "direct raster layer resource");
    const bounds = record(resource.bounds, "direct raster layer bounds");
    if (bounds.kind !== "finite") return null;
    layerBounds.set(node, rect(bounds.rect, "direct raster layer rect"));
  }
  const memo = new Map<string, boolean>();
  const visiting = new Set<number>();
  const eligible = (id: number, depth: number, insideLayer: boolean): boolean => {
    if (depth > 128 || visiting.has(id)) return false;
    const memoKey = `${id}:${Number(insideLayer)}`;
    const cached = memo.get(memoKey);
    if (cached !== undefined) return cached;
    const node = record(requiredIndex(draw.nodes, id, "direct raster node"), "direct raster node");
    if (node.kind !== "group") {
      memo.set(memoKey, true);
      return true;
    }
    const group = record(node.value, "direct raster group");
    const opacity = finiteNumber(group.opacity, "direct raster group opacity");
    const filters = array(group.filters, "direct raster group filters");
    const opensLayer = opacity !== 1 || filters.length !== 0;
    const backdrop = group.backdrop == null ? null : backdrops.get(id);
    const supported = group.mask == null
      && group.shader == null
      && group.glass == null
      && group.glassForeground == null
      && String(group.internalBlend) === "normal"
      && opacity >= 0
      && opacity <= 1
      && (group.backdrop == null || backdrop !== undefined)
      && !(backdrop?.hasLocalInputs && (insideLayer || opensLayer))
      && (opacity === 0 || (opacity === 1 && filters.length === 0) || layerBounds.has(id));
    if (!supported) {
      memo.set(memoKey, false);
      return false;
    }
    visiting.add(id);
    const children = array(group.children, "direct raster group children");
    const result = children.every((child) => eligible(
      positiveIdOrZero(child, "direct raster child"),
      depth + 1,
      insideLayer || opensLayer,
    ));
    visiting.delete(id);
    memo.set(memoKey, result);
    return result;
  };
  const roots = draw.roots.map((root) => positiveIdOrZero(root, "direct raster root"));
  const externalBackdropCount = [...backdrops.values()].filter((backdrop) => backdrop.destination !== null).length;
  return roots.every((root) => eligible(root, 0, false))
    && externalBackdropCount === destinationUses.length
    ? { roots, layerBounds, backdrops }
    : null;
}

function executeDirectRasterProgram(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  arena: SurfaceArena,
  admitted: AdmittedProgram,
  bound: BoundProgramRuntimeSchedule,
  destinations: ImageValue[],
  device: number[],
  localPlan: Wire,
): ImageValue {
  const output = positiveId(localPlan.output, "direct raster output");
  const outputRoi = required(bound.resourceRois, output, "direct raster output ROI");
  if (outputRoi.width === 0 || outputRoi.height === 0) return transparentValue();
  const surface = arena.transient(outputRoi.width, outputRoi.height);
  try {
    const canvas = surface.getCanvas();
    canvas.clear(CanvasKit.TRANSPARENT);
    canvas.save();
    canvas.translate(-outputRoi.x, -outputRoi.y);
    canvas.concat(programMatrix(admitted, localPlan, output, device));
    try {
      for (const root of admitted.directRaster!.roots) {
        drawProgramTree(
          CanvasKit,
          builtins,
          arena,
          surface,
          canvas,
          admitted,
          destinations,
          outputRoi,
          root,
          0,
        );
      }
    } finally {
      canvas.restore();
    }
    surface.flush();
    return { image: surface.makeImageSnapshot(), owned: true, roi: outputRoi };
  } finally {
    arena.releaseTransient(surface);
  }
}

function drawProgramTree(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  arena: SurfaceArena,
  surface: Surface,
  canvas: Canvas,
  admitted: AdmittedProgram,
  destinations: ImageValue[],
  outputRoi: DeviceRoi,
  nodeId: number,
  depth: number,
): void {
  if (depth > 128) fail("program_schedule", "direct raster tree exceeds the group depth budget");
  const node = record(requiredIndex(admitted.draw.nodes, nodeId, "direct raster node"), "direct raster node");
  if (node.kind !== "group") {
    drawProgramNode(CanvasKit, builtins, canvas, admitted, nodeId, IDENTITY_MATRIX);
    return;
  }
  const group = record(node.value, "direct raster group");
  const opacity = finiteNumber(group.opacity, "direct raster group opacity");
  const filters = array(group.filters, "direct raster group filters");
  canvas.save();
  let layer = false;
  let layerPaint: Paint | null = null;
  let layerFilter: ImageFilter | null = null;
  try {
    canvas.concat(numberArray(group.transform, 9, "direct raster group transform"));
    if (opacity === 0) return;
    if (opacity !== 1 || filters.length !== 0) {
      layerPaint = new CanvasKit.Paint();
      layerPaint.setAlphaf(opacity);
      const bounds = required(
        admitted.directRaster!.layerBounds,
        nodeId,
        "direct raster layer bounds",
      );
      for (const rawFilter of filters) {
        const next = drawProgramImageFilter(
          CanvasKit,
          record(rawFilter, "direct raster group filter"),
          layerFilter,
        );
        if (next !== layerFilter) layerFilter?.delete();
        layerFilter = next;
      }
      if (layerFilter) layerPaint.setImageFilter(layerFilter);
      canvas.saveLayer(layerPaint, skRect(CanvasKit, bounds));
      layer = true;
    }
    // The layer is opened first so this group's clip limits the filter input without clipping a
    // blur/drop-shadow expansion when the layer is restored. Ancestor clips remain outside and
    // therefore continue to constrain the complete descendant result.
    if (group.clip != null) {
      applyClip(CanvasKit, canvas, admitted.draw, record(group.clip, "direct raster group clip"), IDENTITY_MATRIX);
    }
    if (group.backdrop != null) {
      const contract = required(
        admitted.directRaster!.backdrops,
        nodeId,
        "direct raster backdrop contract",
      );
      const destination = contract.destination === null ? null : destinations[contract.destination];
      if (contract.destination !== null && !destination) {
        fail("program_schedule", `direct raster backdrop destination ${contract.destination + 1} is absent`);
      }
      drawDirectRasterBackdrop(
        CanvasKit,
        arena,
        surface,
        canvas,
        destination ?? null,
        contract.hasLocalInputs,
        record(group.backdrop, "direct raster backdrop"),
        outputRoi,
      );
    }
    for (const child of array(group.children, "direct raster group children")) {
      drawProgramTree(
        CanvasKit,
        builtins,
        arena,
        surface,
        canvas,
        admitted,
        destinations,
        outputRoi,
        positiveIdOrZero(child, "direct raster child"),
        depth + 1,
      );
    }
  } finally {
    if (layer) canvas.restore();
    layerPaint?.delete();
    layerFilter?.delete();
    canvas.restore();
  }
}

function drawDirectRasterBackdrop(
  CanvasKit: CanvasKit,
  arena: SurfaceArena,
  surface: Surface,
  canvas: Canvas,
  destination: ImageValue | null,
  hasLocalInputs: boolean,
  backdrop: Wire,
  outputRoi: DeviceRoi,
): void {
  const bounds = rect(backdrop.bounds, "direct raster backdrop bounds");
  const filters = array(backdrop.filters, "direct raster backdrop filters");
  let localSnapshot: Image | null = null;
  let combinedSnapshot: Image | null = null;
  let combinedSurface: Surface | null = null;
  let input = destination;
  if (hasLocalInputs) {
    surface.flush();
    localSnapshot = surface.makeImageSnapshot();
    const local: ImageValue = { image: localSnapshot, owned: false, roi: outputRoi };
    if (destination) {
      combinedSurface = arena.transient(outputRoi.width, outputRoi.height);
      const combinedCanvas = combinedSurface.getCanvas();
      combinedCanvas.clear(CanvasKit.TRANSPARENT);
      combinedCanvas.save();
      combinedCanvas.translate(-outputRoi.x, -outputRoi.y);
      try {
        drawImageValue(CanvasKit, combinedCanvas, destination, 1, "src");
        drawImageValue(CanvasKit, combinedCanvas, local, 1, "srcOver");
      } finally {
        combinedCanvas.restore();
      }
      combinedSurface.flush();
      combinedSnapshot = combinedSurface.makeImageSnapshot();
      input = { image: combinedSnapshot, owned: false, roi: outputRoi };
    } else {
      input = local;
    }
  }
  if (!input) fail("program_schedule", "direct raster backdrop has no destination input");
  canvas.save();
  try {
    // Establish the local-space output clip first, then cancel the complete local-to-surface
    // matrix so the resolved destination remains stationary in root device coordinates. Skia's
    // clip is already stored in device space, therefore filtering may still sample the admitted
    // destination outside the output bounds while only the requested backdrop region is written.
    canvas.clipRect(skRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, true);
    const inverse = inverse3(numberArray(canvas.getTotalMatrix(), 9, "direct raster canvas matrix"));
    if (!inverse) fail("program_schedule", "direct raster backdrop has a non-invertible transform");
    canvas.concat(inverse);
    canvas.translate(-outputRoi.x, -outputRoi.y);
    if (filters.length === 0) {
      drawImageValue(CanvasKit, canvas, input, 1, "src");
    } else {
      applyFiltersAsLayerValue(CanvasKit, canvas, input, filters, null);
    }
  } finally {
    canvas.restore();
    combinedSnapshot?.delete();
    if (combinedSurface) arena.releaseTransient(combinedSurface);
    localSnapshot?.delete();
  }
}

function executeProgramPass(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  glassKernel: CanvasKitGlassKernel | null,
  canvas: Canvas,
  admitted: AdmittedProgram,
  kind: Wire,
  values: Map<number, ImageValue>,
  destinations: ImageValue[],
  device: number[],
  localPlan: Wire,
  outputRoi: DeviceRoi,
): void {
  switch (kind.kind) {
    case "clear": return;
    case "rasterNode":
      drawProgramNode(CanvasKit, builtins, canvas, admitted, positiveIdOrZero(kind.node, "RasterNode.node"), programMatrix(admitted, localPlan, kind.output, device));
      return;
    case "rasterTree": {
      const matrix = programMatrix(admitted, localPlan, kind.output, device);
      for (const root of array(kind.roots, "RasterTree.roots")) {
        drawProgramNode(CanvasKit, builtins, canvas, admitted, positiveIdOrZero(root, "RasterTree.root"), matrix);
      }
      return;
    }
    case "readDestination": {
      const external = kind.external == null
        ? transparentValue()
        : destinations[positiveId(kind.external, "program destination") - 1] ?? transparentValue();
      drawImageValue(CanvasKit, canvas, external, 1, "src");
      for (const input of array(kind.localInputs, "local destination inputs")) {
        drawImageValue(CanvasKit, canvas, programValue(values, input), 1, "srcOver");
      }
      return;
    }
    case "backdrop": {
      const bounds = rect(kind.bounds, "program backdrop bounds");
      const input = programValue(values, kind.input);
      canvas.save();
      canvas.clipRect(skRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, true);
      drawImageValue(CanvasKit, canvas, input, 1, "src");
      canvas.restore();
      applyFiltersAsLayerValue(CanvasKit, canvas, input, array(kind.filters, "backdrop filters"), bounds);
      return;
    }
    case "motionGlass": {
      const kernel = requireGlassKernel(glassKernel);
      const input = programValue(values, kind.input);
      const ownerToDevice = programMatrix(admitted, localPlan, kind.output, device);
      const uniforms = kernel.pack_motion_glass_uniforms(
        JSON.stringify(record(kind.program, "MotionGlassProgram")),
        Float64Array.from(ownerToDevice),
      );
      drawMotionGlassShader(CanvasKit, builtins, canvas, input, outputRoi, uniforms);
      return;
    }
    case "applyMotionGlassForeground": {
      const kernel = requireGlassKernel(glassKernel);
      const input = programValue(values, kind.input);
      const ownerToDevice = programTransformMatrix(
        admitted,
        numberArray(kind.ownerToProgram, 9, "MotionGlassForeground.ownerToProgram"),
        device,
      );
      const uniforms = kernel.pack_motion_glass_foreground_uniforms(
        JSON.stringify(record(kind.program, "MotionGlassForegroundProgram")),
        Float64Array.from(ownerToDevice),
      );
      drawMotionGlassShader(CanvasKit, builtins, canvas, input, outputRoi, uniforms);
      return;
    }
    case "sourceOver":
      drawImageValue(CanvasKit, canvas, programValue(values, kind.destination), 1, "src");
      drawImageValue(CanvasKit, canvas, programValue(values, kind.source), 1, "srcOver");
      return;
    case "applyClip":
      canvas.save();
      applyClip(CanvasKit, canvas, admitted.draw, record(kind.clip, "program clip"), programMatrix(admitted, localPlan, kind.output, device));
      drawImageValue(CanvasKit, canvas, programValue(values, kind.input), 1, "src");
      canvas.restore();
      return;
    case "applyFilter": {
      const matrix = programMatrix(admitted, localPlan, kind.output, device);
      applyFiltersAsLayerValue(
        CanvasKit,
        canvas,
        programValue(values, kind.input),
        [programFilterInDeviceSpace(record(kind.filter, "program filter"), matrix)],
        null,
      );
      return;
    }
    case "applyMask":
      drawImageValue(CanvasKit, canvas, programValue(values, kind.input), 1, "src");
      if (kind.mode === "luminance") {
        drawLuminanceMaskValue(CanvasKit, canvas, programValue(values, kind.mask));
      } else {
        drawImageValue(CanvasKit, canvas, programValue(values, kind.mask), 1, "dstIn");
      }
      return;
    case "applyOpacity":
      drawImageValue(CanvasKit, canvas, programValue(values, kind.input), finiteNumber(kind.opacity, "program opacity"), "src");
      return;
    case "applyShader":
      applyProgramShader(CanvasKit, builtins, canvas, admitted, programValue(values, kind.input), record(kind.shader, "program shader"));
      return;
    case "applyTransform":
      drawImageValue(CanvasKit, canvas, programValue(values, kind.input), 1, "src");
      return;
    case "blend":
      drawCreativeBlendValues(
        CanvasKit,
        builtins,
        canvas,
        programValue(values, kind.source),
        programValue(values, kind.destination),
        String(kind.mode),
        1,
        false,
      );
      return;
    default:
      fail("unsupported_program_pass", `program pass '${String(kind.kind)}' is not closed`);
  }
}

function drawProgramNode(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  admitted: AdmittedProgram,
  nodeId: number,
  matrix: number[],
): void {
  const node = record(admitted.draw.nodes[nodeId], `DrawProgram node ${nodeId}`);
  canvas.save();
  canvas.concat(matrix);
  try {
    switch (node.kind) {
      case "path": drawPathNode(CanvasKit, canvas, admitted.draw, record(node.value, "path node")); break;
      case "geometryBatch": drawBatchNode(CanvasKit, canvas, admitted.draw, record(node.value, "batch node")); break;
      case "image": drawImageNode(CanvasKit, builtins, canvas, admitted, record(node.value, "image node")); break;
      case "glyphRun": drawGlyphNode(CanvasKit, canvas, admitted, record(node.value, "glyph node")); break;
      case "shadow": drawShadowNode(CanvasKit, canvas, record(node.value, "shadow node")); break;
      case "runtimeShader": drawRuntimeShaderNode(CanvasKit, builtins, canvas, admitted, record(node.value, "runtime shader node")); break;
      case "scene3d": drawSceneNode(CanvasKit, builtins, canvas, admitted, record(node.value, "Scene3D node")); break;
      case "group": fail("program_schedule", `group node ${nodeId} entered RasterNode`);
      default: fail("unsupported_node", `DrawProgram node '${String(node.kind)}' is not closed`);
    }
  } finally {
    canvas.restore();
  }
}

function drawPathNode(CanvasKit: CanvasKit, canvas: Canvas, draw: DrawProgramWire, node: Wire): void {
  const path = buildPath(CanvasKit, requiredIndex(draw.paths, node.path, "path"), String(node.fillRule));
  try {
    if (node.fill != null) {
      const paint = makePaint(CanvasKit, requiredIndex(draw.paints, node.fill, "paint"));
      try { canvas.drawPath(path, paint); } finally { paint.delete(); }
    }
    if (node.stroke != null) {
      const stroke = record(node.stroke, "path stroke");
      const paint = makePaint(CanvasKit, requiredIndex(draw.paints, stroke.paint, "stroke paint"));
      try {
        const effect = configureStrokePaint(CanvasKit, paint, stroke);
        try { canvas.drawPath(path, paint); }
        finally { effect?.delete(); }
      } finally { paint.delete(); }
    }
  } finally { path.delete(); }
}

function drawBatchNode(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  program: DrawProgramWire,
  node: Wire,
): void {
  const paint = new CanvasKit.Paint();
  try {
    paint.setAntiAlias(true);
    const range = record(node.instances, "batch instance range");
    const start = positiveIdOrZero(range.start, "batch instance start");
    const count = positiveIdOrZero(range.count, "batch instance count");
    const end = start + count;
    if (!Number.isSafeInteger(end) || end > program.batchInstances.count) {
      fail("wire_shape", "batch instance range escapes the DrawProgram table");
    }
    const view = program.batchInstances.view;
    for (let index = start; index < end; index += 1) {
      const offset = index * BATCH_INSTANCE_BYTES;
      const x = view.getFloat64(offset, true);
      const y = view.getFloat64(offset + 8, true);
      const width = view.getFloat64(offset + 16, true);
      const height = view.getFloat64(offset + 24, true);
      const alpha = view.getFloat32(offset + 44, true);
      paint.setColor(alpha <= 0
        ? CanvasKit.Color4f(0, 0, 0, 0)
        : CanvasKit.Color4f(
            view.getFloat32(offset + 32, true) / alpha,
            view.getFloat32(offset + 36, true) / alpha,
            view.getFloat32(offset + 40, true) / alpha,
            alpha,
          ));
      if (node.geometry === "circle") canvas.drawCircle(x, y, Math.min(width, height) * .5, paint);
      else canvas.drawRect(CanvasKit.XYWHRect(x, y, width, height), paint);
    }
  } finally { paint.delete(); }
}

function drawImageNode(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  admitted: AdmittedProgram,
  node: Wire,
): void {
  const texture = record(node.texture, "image texture");
  const object = resolveProgramTexture(admitted.textures, texture, "program texture");
  const image = object.image!;
  const src = rect(node.src, "image src");
  const dst = rect(node.dst, "image dst");
  const source = {
    x: src.x * image.width(),
    y: src.y * image.height(),
    width: src.width * image.width(),
    height: src.height * image.height(),
  };
  const localFromPixel = [
    dst.width / source.width, 0, dst.x - source.x * dst.width / source.width,
    0, dst.height / source.height, dst.y - source.y * dst.height / source.height,
    0, 0, 1,
  ];
  const shader = normalizedExternalShader(CanvasKit, builtins, object, String(node.sampling));
  const paint = new CanvasKit.Paint();
  try {
    paint.setAlphaf(finiteNumber(node.opacity, "image opacity"));
    paint.setShader(shader);
    canvas.save();
    canvas.clipRect(skRect(CanvasKit, dst), CanvasKit.ClipOp.Intersect, true);
    canvas.concat(localFromPixel);
    canvas.drawRect(skRect(CanvasKit, source), paint);
    canvas.restore();
  } finally { paint.delete(); shader.delete(); }
}

function drawGlyphNode(CanvasKit: CanvasKit, canvas: Canvas, admitted: AdmittedProgram, node: Wire): void {
  const fontKeyWire = record(node.font, "glyph font");
  const face = required(admitted.fonts, fontKey(String(fontKeyWire.faceHash), positiveIdOrZero(fontKeyWire.faceIndex, "faceIndex")), "font face");
  const font = new CanvasKit.Font(face, finiteNumber(node.fontSize, "font size"));
  try {
    // Match the Native Skia executor exactly. CanvasKit otherwise keeps its platform defaults,
    // which changes stem coverage (and therefore parity) even with identical glyph positions.
    configureGlyphCoverage(CanvasKit, font);
    const glyphs = array(node.glyphs, "glyphs");
    const ids = Uint16Array.from(glyphs.map((raw) => positiveIdOrZero(record(raw, "glyph").id, "glyph id")));
    const positions = Float32Array.from(glyphs.flatMap((raw) => {
      const glyph = record(raw, "glyph");
      return [finiteNumber(glyph.x, "glyph x"), finiteNumber(glyph.y, "glyph y")];
    }));
    const fill = makePaint(CanvasKit, requiredIndex(admitted.draw.paints, node.paint, "glyph paint"));
    try { canvas.drawGlyphs(ids, positions, 0, 0, font, fill); }
    finally { fill.delete(); }
    if (node.stroke != null) {
      const stroke = record(node.stroke, "glyph stroke");
      const outline = makePaint(CanvasKit, requiredIndex(admitted.draw.paints, stroke.paint, "glyph stroke paint"));
      try {
        const effect = configureStrokePaint(CanvasKit, outline, stroke);
        try { canvas.drawGlyphs(ids, positions, 0, 0, font, outline); }
        finally { effect?.delete(); }
      } finally { outline.delete(); }
    }
  } finally { font.delete(); }
}

function configureGlyphCoverage(CanvasKit: CanvasKit, font: Font): void {
  const profile = CANVASKIT_GLYPH_COVERAGE_PROFILE;
  if (profile.hinting === "none") font.setHinting(CanvasKit.FontHinting.None);
  if (profile.edging === "antialias") font.setEdging(CanvasKit.FontEdging.AntiAlias);
  font.setSubpixel(profile.subpixelPositioning);
}

function drawShadowNode(CanvasKit: CanvasKit, canvas: Canvas, node: Wire): void {
  const original = record(node.shape, "shadow shape");
  const shapeBounds = rect(original.rect, "shadow shape bounds");
  const offset = pair(node.offset, "shadow offset");
  const spread = finiteNumber(node.spread, "shadow spread");
  const radii = array(original.radii, "shadow radii").map((raw) => {
    const radius = pair(raw, "shadow radius");
    return [Math.max(0, radius[0] + spread), Math.max(0, radius[1] + spread)];
  });
  const shape = {
    rect: {
      x: shapeBounds.x + offset[0] - spread,
      y: shapeBounds.y + offset[1] - spread,
      width: shapeBounds.width + spread * 2,
      height: shapeBounds.height + spread * 2,
    },
    radii,
  };
  const paint = new CanvasKit.Paint();
  const sigma = Math.max(
    finiteNumber(node.sigmaX, "shadow sigmaX"),
    finiteNumber(node.sigmaY, "shadow sigmaY"),
  );
  const filter = sigma > 0
    ? CanvasKit.MaskFilter.MakeBlur(CanvasKit.BlurStyle.Normal, sigma, false)
    : null;
  try {
    paint.setAntiAlias(true);
    paint.setColor(linearColor(CanvasKit, record(node.color, "shadow color") as unknown as LinearColorWire));
    if (filter) paint.setMaskFilter(filter);
    if (node.inset === true) {
      canvas.save();
      canvas.clipRRect(roundRect(CanvasKit, original), CanvasKit.ClipOp.Intersect, true);
      canvas.drawRRect(roundRect(CanvasKit, shape), paint);
      canvas.restore();
    } else {
      canvas.drawRRect(roundRect(CanvasKit, shape), paint);
    }
  } finally { filter?.delete(); paint.delete(); }
}

function drawRuntimeShaderNode(CanvasKit: CanvasKit, builtins: CanvasKitBuiltinRuntime, canvas: Canvas, admitted: AdmittedProgram, node: Wire): void {
  const shader = record(node.shader, "runtime shader key");
  const effect = required(admitted.shaders, String(shader.uri), "runtime shader");
  const bounds = rect(node.bounds, "runtime shader bounds");
  const runtime = instantiateRuntimeShader(CanvasKit, builtins, effect, array(node.uniforms, "shader uniforms"), array(node.textures, "shader textures"), admitted, null);
  const paint = new CanvasKit.Paint();
  try { paint.setShader(runtime); canvas.drawRect(skRect(CanvasKit, bounds), paint); }
  finally { paint.delete(); runtime.delete(); }
}

function drawSceneNode(CanvasKit: CanvasKit, builtins: CanvasKitBuiltinRuntime, canvas: Canvas, admitted: AdmittedProgram, node: Wire): void {
  const scene = record(node.scene, "Scene3D key");
  const object = required(admitted.scenes, String(scene.contentHash), "Scene3D image");
  const image = object.image!;
  const bounds = rect(node.bounds, "Scene3D bounds");
  const shader = normalizedExternalShader(CanvasKit, builtins, object);
  const paint = new CanvasKit.Paint();
  const localFromPixel = [
    bounds.width / image.width(), 0, bounds.x,
    0, bounds.height / image.height(), bounds.y,
    0, 0, 1,
  ];
  try {
    paint.setShader(shader);
    canvas.save();
    canvas.clipRect(skRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, true);
    canvas.concat(localFromPixel);
    canvas.drawRect(CanvasKit.XYWHRect(0, 0, image.width(), image.height()), paint);
    canvas.restore();
  } finally { paint.delete(); shader.delete(); }
}

function applyProgramShader(CanvasKit: CanvasKit, builtins: CanvasKitBuiltinRuntime, canvas: Canvas, admitted: AdmittedProgram, input: ImageValue, shaderWire: Wire): void {
  const key = record(shaderWire.shader, "group shader key");
  const effect = required(admitted.shaders, String(key.uri), "group shader");
  const runtime = instantiateRuntimeShader(CanvasKit, builtins, effect, array(shaderWire.uniforms, "group uniforms"), array(shaderWire.textures, "group textures"), admitted, input);
  const paint = new CanvasKit.Paint();
  try { paint.setShader(runtime); canvas.drawRect(skRect(CanvasKit, rect(shaderWire.bounds, "group shader bounds")), paint); }
  finally { paint.delete(); runtime.delete(); }
}

function instantiateRuntimeShader(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  effect: RuntimeEffect,
  uniforms: unknown[],
  textures: unknown[],
  admitted: AdmittedProgram,
  content: ImageValue | null,
): Shader {
  const scalars: number[] = [];
  for (const raw of uniforms) {
    const binding = record(raw, "shader uniform");
    const value = record(binding.value, "shader uniform value");
    if (value.kind === "float") scalars.push(finiteNumber(value.value, "shader float"));
    else if (value.kind === "float2") scalars.push(...pair(value.value, "shader float2"));
    else if (value.kind === "color") {
      const color = record(value.value, "shader color");
      scalars.push(finiteNumber(color.red, "red"), finiteNumber(color.green, "green"), finiteNumber(color.blue, "blue"), finiteNumber(color.alpha, "alpha"));
    } else if (value.kind === "bool") scalars.push(value.value === true ? 1 : 0);
    else fail("shader_abi", `unknown shader uniform kind '${String(value.kind)}'`);
  }
  const children: Shader[] = [];
  try {
    if (content?.image) children.push(imageValueShader(CanvasKit, content, "linearClamp"));
    for (const raw of textures) {
      const binding = record(raw, "shader texture binding");
      const texture = record(binding.texture, "shader texture key");
      children.push(normalizedExternalShader(
        CanvasKit,
        builtins,
        resolveProgramTexture(admitted.textures, texture, "shader texture"),
        String(binding.sampling),
      ));
    }
    const shader = effect.makeShaderWithChildren(Float32Array.from(scalars), children);
    if (!shader) fail("shader_abi", "RuntimeEffect child/uniform ABI mismatch");
    return shader;
  } finally {
    for (const child of children) child.delete();
  }
}

function executeKernel(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  arena: SurfaceArena,
  canvas: Canvas,
  invocation: Wire,
  resources: Map<number, ImageValue>,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
): void {
  switch (invocation.kind) {
    case "group": drawImage(CanvasKit, canvas, image(resources, invocation.input), 1, "src"); return;
    case "mask":
      drawImage(CanvasKit, canvas, image(resources, invocation.input), 1, "src");
      applyPreparedMask(CanvasKit, arena, canvas, record(invocation.mask, "prepared mask"));
      return;
    case "filter":
    case "adjustmentEffect":
      applyPreparedEffect(CanvasKit, builtins, canvas, image(resources, invocation.input), record(invocation.effect, "effect"), dynamic, extent);
      return;
    case "transition": {
      drawImage(CanvasKit, canvas, image(resources, invocation.backdrop), 1, "src");
      const progress = dynamicScalar(dynamic, invocation.progress);
      const kernel = transitionKernel(String(invocation.kernel));
      const from = opacityShader(
        CanvasKit,
        builtins,
        image(resources, invocation.from),
        dynamicScalar(dynamic, invocation.fromOpacity),
      );
      const to = opacityShader(
        CanvasKit,
        builtins,
        image(resources, invocation.to),
        dynamicScalar(dynamic, invocation.toOpacity),
      );
      try {
        const transition = builtins.shader(
          kernel,
          [extent.width, extent.height, progress],
          [from, to],
        );
        try { drawShader(CanvasKit, canvas, transition, "srcOver"); }
        finally { transition.delete(); }
      } finally {
        from.delete();
        to.delete();
      }
      return;
    }
    default: fail("unsupported_kernel", `kernel '${String(invocation.kind)}' is not closed`);
  }
}

/** @internal Exported only for backend pixel-contract tests; package entrypoints do not re-export it. */
export function applyPreparedEffect(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  input: Image | null,
  effect: Wire,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
): void {
  const kernel = record(effect.kernel, "effect kernel");
  const region = effectRegion(kernel);
  const space = record(effect.space, "effect space");
  const restricted = space.kind === "layer" || region !== null;
  if (restricted) {
    drawImage(CanvasKit, canvas, input, 1, "src");
    canvas.save();
    applyEffectClip(CanvasKit, canvas, effect, region, dynamic, extent);
  }
  try {
    drawPreparedEffect(CanvasKit, builtins, canvas, input, effect, kernel, dynamic, extent);
  } finally {
    if (restricted) canvas.restore();
  }
}

function drawPreparedEffect(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  input: Image | null,
  effect: Wire,
  kernel: Wire,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
): void {
  if (kernel.kind === "gaussianBlur") {
    const sigma = dynamicScalar(dynamic, kernel.sigmaDevicePx);
    const filter = CanvasKit.ImageFilter.MakeBlur(sigma, sigma, CanvasKit.TileMode.Decal, null);
    const paint = new CanvasKit.Paint();
    try { paint.setImageFilter(filter); drawImageWithPaint(CanvasKit, canvas, input, paint, "src"); }
    finally { filter?.delete(); paint.delete(); }
    return;
  }
  if (kernel.kind === "directionalBlur") {
    const direction = kernel.axis === "horizontal" ? [1, 0] : [0, 1];
    drawBuiltinImage(
      CanvasKit,
      builtins,
      canvas,
      "directionalBlur",
      [direction[0]!, direction[1]!, dynamicScalar(dynamic, kernel.spanDevicePx)],
      [input],
      "src",
    );
    return;
  }
  if (kernel.kind === "colorGrade") {
    drawBuiltinImage(
      CanvasKit,
      builtins,
      canvas,
      "colorGrade",
      [
        extent.width,
        extent.height,
        finiteNumber(kernel.brightness, "color grade brightness"),
        finiteNumber(kernel.contrast, "color grade contrast"),
        finiteNumber(kernel.saturation, "color grade saturation"),
        finiteNumber(kernel.temperature, "color grade temperature"),
        finiteNumber(kernel.vignette, "color grade vignette"),
      ],
      [input],
      "src",
    );
    return;
  }
  if (kernel.kind === "mosaic") {
    const anchor = effectAnchor(effect, dynamic);
    drawBuiltinImage(
      CanvasKit,
      builtins,
      canvas,
      "mosaic",
      [extent.width, extent.height, dynamicScalar(dynamic, kernel.blockSizeDevicePx), anchor[0], anchor[1]],
      [input],
      "src",
    );
    return;
  }
  if (kernel.kind === "spotlight") {
    const center = pair(kernel.center, "spotlight center");
    drawBuiltinImage(
      CanvasKit,
      builtins,
      canvas,
      "spotlight",
      [
        extent.width,
        extent.height,
        center[0],
        center[1],
        finiteNumber(kernel.radius, "spotlight radius"),
        finiteNumber(kernel.feather, "spotlight feather"),
        finiteNumber(kernel.intensity, "spotlight intensity"),
      ],
      [input],
      "src",
    );
    return;
  }
  fail("unsupported_kernel", `prepared effect '${String(kernel.kind)}' is not valid in this pass`);
}

function applyEffectClip(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  effect: Wire,
  region: Wire | null,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
): void {
  const space = record(effect.space, "effect space");
  if (space.kind === "root") {
    if (region) {
      const value = unitRegion(region);
      canvas.clipRect(
        CanvasKit.XYWHRect(
          value.x * extent.width,
          value.y * extent.height,
          value.width * extent.width,
          value.height * extent.height,
        ),
        CanvasKit.ClipOp.Intersect,
        false,
      );
    }
    return;
  }
  if (space.kind !== "layer") fail("effect_space", `effect space '${String(space.kind)}' is outside the closed set`);
  canvas.clipRect(deviceRect(CanvasKit, dynamicBounds(dynamic, space.bounds)), CanvasKit.ClipOp.Intersect, false);
  if (!region) return;

  const transform = dynamicTransform(dynamic, space.transform);
  const inverse = inverse3(transform);
  if (!inverse) fail("effect_space", `effect '${String(effect.semanticPath)}' has a non-invertible layer transform`);
  const value = unitRegion(region);
  canvas.concat(transform);
  canvas.clipRect(CanvasKit.XYWHRect(value.x, value.y, value.width, value.height), CanvasKit.ClipOp.Intersect, false);
  canvas.concat(inverse);
}

function effectRegion(kernel: Wire): Wire | null {
  if (kernel.kind !== "gaussianBlur" && kernel.kind !== "mosaic") return null;
  return kernel.region == null ? null : record(kernel.region, "effect region");
}

function unitRegion(region: Wire): RectWire {
  const value = rect(region, "effect region");
  if (value.x < 0 || value.y < 0 || value.width <= 0 || value.height <= 0 || value.x + value.width > 1 || value.y + value.height > 1) {
    fail("effect_region", "effect region must be a non-empty normalized unit rectangle");
  }
  return value;
}

function effectAnchor(effect: Wire, dynamic: Map<number, Wire>): [number, number] {
  const space = record(effect.space, "effect space");
  if (space.kind === "root") return [0, 0];
  if (space.kind !== "layer") fail("effect_space", `effect space '${String(space.kind)}' is outside the closed set`);
  const anchor = project3(dynamicTransform(dynamic, space.transform), [0, 0]);
  if (!anchor) fail("effect_space", `effect '${String(effect.semanticPath)}' has an invalid layer origin`);
  return anchor;
}

function applyPreparedMask(CanvasKit: CanvasKit, arena: SurfaceArena, canvas: Canvas, mask: Wire): void {
  const transform = record(mask.deviceFromMask, "mask transform");
  const surface = arena.transient();
  const maskCanvas = surface.getCanvas();
  const shape = new CanvasKit.Paint();
  let snapshot: Image | null = null;
  try {
    maskCanvas.clear(mask.invert ? CanvasKit.WHITE : CanvasKit.TRANSPARENT);
    shape.setAntiAlias(true);
    shape.setBlendMode(CanvasKit.BlendMode.Src);
    shape.setColor(mask.invert ? CanvasKit.TRANSPARENT : CanvasKit.WHITE);
    maskCanvas.save();
    maskCanvas.concat(numberArray(transform.matrix, 9, "mask matrix"));
    if (mask.shape === "ellipse") maskCanvas.drawOval(CanvasKit.XYWHRect(0, 0, 1, 1), shape);
    else maskCanvas.drawRect(CanvasKit.XYWHRect(0, 0, 1, 1), shape);
    maskCanvas.restore();
    surface.flush();
    snapshot = surface.makeImageSnapshot();
  } finally {
    shape.delete();
    arena.releaseTransient(surface);
  }
  const coverage = new CanvasKit.Paint();
  const sigma = finiteNumber(mask.featherSigmaDevicePx, "mask feather sigma");
  const blur = sigma > 0
    ? CanvasKit.ImageFilter.MakeBlur(sigma, sigma, CanvasKit.TileMode.Decal, null)
    : null;
  try {
    coverage.setBlendMode(CanvasKit.BlendMode.DstIn);
    if (blur) coverage.setImageFilter(blur);
    canvas.drawImage(snapshot, 0, 0, coverage);
  } finally {
    blur?.delete();
    coverage.delete();
    snapshot.delete();
  }
}

function renderImport(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  sourceObject: CanvasKitExternalObject,
  pass: Wire,
  dynamic: Map<number, Wire>,
  externals: Map<number, CanvasKitExternalObject>,
): void {
  const source = sourceObject.image!;
  const placement = record(pass.placement, "external placement");
  const transform = dynamicTransform(dynamic, pass.transform);
  const bounds = dynamicBounds(dynamic, pass.bounds);
  canvas.save();
  canvas.clipRect(deviceRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, false);
  const backdrop = placement.backdrop == null ? null : record(placement.backdrop, "external backdrop");
  if (backdrop?.kind === "color") {
    canvas.save(); canvas.concat(transform);
    const paint = new CanvasKit.Paint();
    try { paint.setColor(color4(backdrop.workingLinearRec2020Premul)); canvas.drawRect(CanvasKit.XYWHRect(0, 0, 1, 1), paint); }
    finally { paint.delete(); canvas.restore(); }
  } else if (backdrop?.kind === "blur") {
    const sigma = dynamicScalar(dynamic, backdrop.sigmaDevicePx);
    const blur = CanvasKit.ImageFilter.MakeBlur(sigma, sigma, CanvasKit.TileMode.Decal, null);
    const paint = new CanvasKit.Paint();
    const shader = normalizedExternalShader(CanvasKit, builtins, sourceObject);
    try {
      paint.setImageFilter(blur);
      drawExternalSample(
        CanvasKit,
        canvas,
        source,
        record(backdrop.sample, "external blur sample"),
        rect(placement.clipRect, "blur destination"),
        rect(placement.clipRect, "blur clip"),
        transform,
        paint,
        shader,
      );
    } finally {
      shader.delete();
      paint.delete();
      blur.delete();
    }
  }
  const sourcePipeline = record(pass.sourcePipeline, "source pipeline");
  let sourceShader = normalizedExternalShader(CanvasKit, builtins, sourceObject);
  if (sourcePipeline.chromaKey != null) {
    const effect = record(sourcePipeline.chromaKey, "chroma key effect");
    const kernel = record(effect.kernel, "chroma key kernel");
    if (kernel.kind !== "chromaKey") fail("source_pipeline", "chroma key slot carries the wrong kernel");
    const key = numberArray(kernel.keyWorkingLinearRec2020, 3, "chroma key color");
    const child = sourceShader;
    try {
      sourceShader = builtins.shader(
        "chromaKey",
        [
          key[0]!,
          key[1]!,
          key[2]!,
          finiteNumber(kernel.intensity, "chroma key intensity"),
          finiteNumber(kernel.shadow, "chroma key shadow"),
          dynamicScalar(dynamic, kernel.featherSigmaDevicePx),
          finiteNumber(kernel.edgeClean, "chroma key edge clean"),
        ],
        [child],
      );
    } finally { child.delete(); }
  }
  try {
    drawExternalSample(
      CanvasKit,
      canvas,
      source,
      record(placement.sample, "external sample"),
      rect(placement.contentRect, "content rect"),
      rect(placement.clipRect, "clip rect"),
      transform,
      undefined,
      sourceShader,
    );
  } finally { sourceShader.delete(); }
  canvas.restore();
}

function drawExternalSample(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  image: Image,
  sample: Wire,
  destination: RectWire,
  clip: RectWire,
  transform: number[],
  paintOverride?: Paint,
  shaderOverride?: Shader,
): void {
  if (sample.kind === "empty") return;
  if (sample.kind !== "texture") fail("external_sample", `unknown sample kind '${String(sample.kind)}'`);
  const textureFromContent = numberArray(sample.textureFromContent, 9, "textureFromContent");
  const inverse = inverse3(textureFromContent);
  if (!inverse) fail("external_sample", "non-invertible texture mapping");
  const normalizedFromPixel = [1 / image.width(), 0, 0, 0, 1 / image.height(), 0, 0, 0, 1];
  const localFromContent = [destination.width, 0, destination.x, 0, destination.height, destination.y, 0, 0, 1];
  const localFromPixel = mul3(localFromContent, mul3(inverse, normalizedFromPixel));
  const sourceBounds = rect(sample.inputSampleBounds, "input sample bounds");
  const src = CanvasKit.XYWHRect(sourceBounds.x * image.width(), sourceBounds.y * image.height(), sourceBounds.width * image.width(), sourceBounds.height * image.height());
  const paint = paintOverride ?? new CanvasKit.Paint();
  try {
    canvas.save();
    canvas.concat(transform);
    canvas.clipRect(skRect(CanvasKit, clip), CanvasKit.ClipOp.Intersect, true);
    canvas.clipRect(skRect(CanvasKit, destination), CanvasKit.ClipOp.Intersect, true);
    canvas.concat(localFromPixel);
    if (shaderOverride) {
      paint.setShader(shaderOverride);
      canvas.drawRect(src, paint);
      paint.setShader(null);
    } else {
      canvas.drawImageRectOptions(image, src, src, CanvasKit.FilterMode.Linear, CanvasKit.MipmapMode.None, paint);
    }
    canvas.restore();
  } finally { if (!paintOverride) paint.delete(); }
}

function applyFiltersAsLayer(CanvasKit: CanvasKit, canvas: Canvas, input: Image | null, filters: unknown[], bounds: RectWire | null): void {
  let current: ImageFilter | null = null;
  try {
    for (const raw of filters) {
      const filter = record(raw, "DrawProgram filter");
      const next = drawProgramImageFilter(CanvasKit, filter, current);
      if (next !== current) current?.delete();
      current = next;
    }
    const paint = new CanvasKit.Paint();
    try { if (current) paint.setImageFilter(current); if (bounds) canvas.clipRect(skRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, true); drawImageWithPaint(CanvasKit, canvas, input, paint, "src"); }
    finally { paint.delete(); }
  } finally { current?.delete(); }
}

function applyFiltersAsLayerValue(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  input: ImageValue,
  filters: unknown[],
  bounds: RectWire | null,
): void {
  let current: ImageFilter | null = null;
  try {
    for (const raw of filters) {
      const filter = record(raw, "DrawProgram filter");
      const next = drawProgramImageFilter(CanvasKit, filter, current);
      if (next !== current) current?.delete();
      current = next;
    }
    const paint = new CanvasKit.Paint();
    try {
      if (current) paint.setImageFilter(current);
      if (bounds) canvas.clipRect(skRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, true);
      drawImageValueWithPaint(CanvasKit, canvas, input, paint, "src");
    } finally {
      paint.delete();
    }
  } finally {
    current?.delete();
  }
}

function drawProgramImageFilter(CanvasKit: CanvasKit, filter: Wire, input: ImageFilter | null): ImageFilter | null {
  if (filter.kind === "blur") {
    const sigmaX = finiteNumber(filter.sigmaX, "sigmaX");
    const sigmaY = finiteNumber(filter.sigmaY, "sigmaY");
    return sigmaX === 0 && sigmaY === 0
      ? input
      : CanvasKit.ImageFilter.MakeBlur(sigmaX, sigmaY, CanvasKit.TileMode.Decal, input);
  }
  const matrix = drawProgramColorMatrix(filter);
  if (matrix) {
    const color = CanvasKit.ColorFilter.MakeMatrix(matrix);
    try { return CanvasKit.ImageFilter.MakeColorFilter(color, input); }
    finally { color.delete(); }
  }
  if (filter.kind === "dropShadow") {
    const offset = pair(filter.offset, "drop shadow offset");
    return CanvasKit.ImageFilter.MakeDropShadow(
      offset[0],
      offset[1],
      finiteNumber(filter.sigmaX, "sigmaX"),
      finiteNumber(filter.sigmaY, "sigmaY"),
      linearColor(CanvasKit, record(filter.color, "drop shadow color") as unknown as LinearColorWire),
      input,
    );
  }
  if (filter.kind === "noiseDisplacement") {
    const frequency = pair(filter.frequency, "noise frequency");
    const octaves = positiveIdOrZero(filter.octaves, "noise octaves");
    const seed = positiveIdOrZero(filter.seed, "noise seed");
    const noise = filter.turbulence === true
      ? CanvasKit.Shader.MakeTurbulence(frequency[0], frequency[1], octaves, seed, 0, 0)
      : CanvasKit.Shader.MakeFractalNoise(frequency[0], frequency[1], octaves, seed, 0, 0);
    const displacement = CanvasKit.ImageFilter.MakeShader(noise);
    noise.delete();
    try {
      return CanvasKit.ImageFilter.MakeDisplacementMap(
        CanvasKit.ColorChannel.Red,
        CanvasKit.ColorChannel.Green,
        finiteNumber(filter.scale, "noise displacement scale"),
        displacement,
        input,
      );
    } finally { displacement.delete(); }
  }
  if (filter.kind === "velocityBlur") {
    const velocity = pair(filter.velocity, "velocity blur vector");
    const span = Math.hypot(velocity[0], velocity[1])
      * Math.min(1, Math.max(0, finiteNumber(filter.shutterAngleDegrees, "shutter angle") / 360));
    if (span <= 1e-6) return input;
    const angle = Math.atan2(velocity[1], velocity[0]);
    const sampling = { filter: CanvasKit.FilterMode.Nearest, mipmap: CanvasKit.MipmapMode.None };
    const rotated = CanvasKit.ImageFilter.MakeMatrixTransform(CanvasKit.Matrix.rotated(-angle), sampling, input);
    const blurred = CanvasKit.ImageFilter.MakeBlur(span / 3, 0, CanvasKit.TileMode.Decal, rotated);
    rotated.delete();
    try {
      return CanvasKit.ImageFilter.MakeMatrixTransform(CanvasKit.Matrix.rotated(angle), sampling, blurred);
    } finally { blurred.delete(); }
  }
  fail("unsupported_filter", `DrawProgram filter '${String(filter.kind)}' has no CanvasKit kernel`);
}

/**
 * Map program-local Blur/DropShadow parameters into a materialized device-space pass.
 * Direct-raster groups keep the local CTM active and do not call this helper. The scheduled
 * fallback has axis-aligned CanvasKit image-filter kernels. Caption similarities are exact; any
 * other affine transform is accepted only when its mapped Gaussian covariance remains
 * axis-aligned.
 */
export function programFilterInDeviceSpace(
  rawFilter: Record<string, unknown>,
  rawLocalToDevice: readonly number[],
): Record<string, unknown> {
  const filter = record(rawFilter, "program filter");
  if (filter.kind !== "blur" && filter.kind !== "dropShadow") return filter;
  const matrix = numberArray(rawLocalToDevice, 9, "program filter local-to-device matrix");
  const sigma = programGaussianInDeviceSpace(
    [finiteNumber(filter.sigmaX, "sigmaX"), finiteNumber(filter.sigmaY, "sigmaY")],
    matrix,
  );
  if (filter.kind === "blur") return { ...filter, sigmaX: sigma[0], sigmaY: sigma[1] };
  const offset = pair(filter.offset, "drop shadow offset");
  return {
    ...filter,
    offset: [
      checkedDeviceFilterValue(matrix[0]! * offset[0] + matrix[1]! * offset[1]),
      checkedDeviceFilterValue(matrix[3]! * offset[0] + matrix[4]! * offset[1]),
    ],
    sigmaX: sigma[0],
    sigmaY: sigma[1],
  };
}

function programGaussianInDeviceSpace(sigma: [number, number], matrix: number[]): [number, number] {
  const [a, b, , d, e, , g, h, i] = matrix as [
    number, number, number, number, number, number, number, number, number,
  ];
  if (matrix.some((value) => !Number.isFinite(value))
    || Math.abs(g) > 1e-12
    || Math.abs(h) > 1e-12
    || Math.abs(i - 1) > 1e-12) {
    fail("unsupported_filter", "program-local spatial filter requires an affine device transform");
  }
  const varianceX = (a * sigma[0]) ** 2 + (b * sigma[1]) ** 2;
  const varianceY = (d * sigma[0]) ** 2 + (e * sigma[1]) ** 2;
  const covariance = a * d * sigma[0] ** 2 + b * e * sigma[1] ** 2;
  const varianceScale = Math.max(varianceX, varianceY, 1);
  if (Math.abs(covariance) > varianceScale * 1e-9) {
    fail("unsupported_filter", "rotated program-local anisotropic blur has no axis-aligned CanvasKit equivalent");
  }
  return [
    checkedDeviceFilterValue(Math.sqrt(varianceX)),
    checkedDeviceFilterValue(Math.sqrt(varianceY)),
  ];
}

function checkedDeviceFilterValue(value: number): number {
  if (!Number.isFinite(value) || Math.abs(value) > 3.4028234663852886e38) {
    fail("unsupported_filter", "program-local spatial filter exceeds the device numeric domain");
  }
  return value;
}

function drawProgramColorMatrix(filter: Wire): number[] | null {
  switch (filter.kind) {
    case "colorMatrix": return numberArray(filter.matrix, 20, "color matrix");
    case "brightness": {
      const amount = finiteNumber(filter.amount, "brightness amount");
      return diagonalColorMatrix(amount, amount, amount, 0);
    }
    case "contrast": {
      const amount = finiteNumber(filter.amount, "contrast amount");
      return diagonalColorMatrix(amount, amount, amount, .5 - .5 * amount);
    }
    case "grayscale": return grayscaleColorMatrix(finiteNumber(filter.amount, "grayscale amount"));
    case "hueRotate": return hueRotateColorMatrix(finiteNumber(filter.degrees, "hue rotation"));
    case "invert": {
      const amount = finiteNumber(filter.amount, "invert amount");
      return diagonalColorMatrix(1 - 2 * amount, 1 - 2 * amount, 1 - 2 * amount, amount);
    }
    case "opacity": {
      const matrix = diagonalColorMatrix(1, 1, 1, 0);
      matrix[18] = finiteNumber(filter.amount, "opacity amount");
      return matrix;
    }
    case "saturate": return saturateColorMatrix(finiteNumber(filter.amount, "saturation amount"));
    case "sepia": return sepiaColorMatrix(finiteNumber(filter.amount, "sepia amount"));
    default: return null;
  }
}

function diagonalColorMatrix(red: number, green: number, blue: number, offset: number): number[] {
  return [red, 0, 0, 0, offset, 0, green, 0, 0, offset, 0, 0, blue, 0, offset, 0, 0, 0, 1, 0];
}

function grayscaleColorMatrix(amount: number): number[] {
  const a = amount;
  return [
    .2126 + .7874 * (1 - a), .7152 - .7152 * (1 - a), .0722 - .0722 * (1 - a), 0, 0,
    .2126 - .2126 * (1 - a), .7152 + .2848 * (1 - a), .0722 - .0722 * (1 - a), 0, 0,
    .2126 - .2126 * (1 - a), .7152 - .7152 * (1 - a), .0722 + .9278 * (1 - a), 0, 0,
    0, 0, 0, 1, 0,
  ];
}

function saturateColorMatrix(amount: number): number[] {
  return [
    .213 + .787 * amount, .715 - .715 * amount, .072 - .072 * amount, 0, 0,
    .213 - .213 * amount, .715 + .285 * amount, .072 - .072 * amount, 0, 0,
    .213 - .213 * amount, .715 - .715 * amount, .072 + .928 * amount, 0, 0,
    0, 0, 0, 1, 0,
  ];
}

function hueRotateColorMatrix(degrees: number): number[] {
  const radians = degrees * Math.PI / 180;
  const cosine = Math.cos(radians);
  const sine = Math.sin(radians);
  return [
    .213 + cosine * .787 - sine * .213, .715 - cosine * .715 - sine * .715, .072 - cosine * .072 + sine * .928, 0, 0,
    .213 - cosine * .213 + sine * .143, .715 + cosine * .285 + sine * .140, .072 - cosine * .072 - sine * .283, 0, 0,
    .213 - cosine * .213 - sine * .787, .715 - cosine * .715 + sine * .715, .072 + cosine * .928 + sine * .072, 0, 0,
    0, 0, 0, 1, 0,
  ];
}

function sepiaColorMatrix(amount: number): number[] {
  const inverse = 1 - amount;
  return [
    .393 * amount + inverse, .769 * amount, .189 * amount, 0, 0,
    .349 * amount, .686 * amount + inverse, .168 * amount, 0, 0,
    .272 * amount, .534 * amount, .131 * amount + inverse, 0, 0,
    0, 0, 0, 1, 0,
  ];
}

function drawLuminanceMask(CanvasKit: CanvasKit, canvas: Canvas, mask: Image | null): void {
  if (!mask) return;
  const color = CanvasKit.ColorFilter.MakeLuma();
  const paint = new CanvasKit.Paint();
  try {
    paint.setColorFilter(color);
    paint.setBlendMode(CanvasKit.BlendMode.DstIn);
    canvas.drawImage(mask, 0, 0, paint);
  } finally {
    paint.delete();
    color.delete();
  }
}

function drawLuminanceMaskValue(CanvasKit: CanvasKit, canvas: Canvas, mask: ImageValue): void {
  if (!mask.image) return;
  const color = CanvasKit.ColorFilter.MakeLuma();
  const paint = new CanvasKit.Paint();
  try {
    paint.setColorFilter(color);
    drawImageValueWithPaint(CanvasKit, canvas, mask, paint, "dstIn");
  } finally {
    paint.delete();
    color.delete();
  }
}

function applyClip(CanvasKit: CanvasKit, canvas: Canvas, draw: DrawProgramWire, clip: Wire, matrix: number[]): void {
  canvas.concat(matrix);
  if (clip.kind === "rect") canvas.clipRect(skRect(CanvasKit, rect(clip.value, "clip rect")), CanvasKit.ClipOp.Intersect, true);
  else if (clip.kind === "roundRect") canvas.clipRRect(roundRect(CanvasKit, record(clip.value, "round clip")), CanvasKit.ClipOp.Intersect, true);
  else if (clip.kind === "path") {
    const value = record(clip.value, "path clip");
    const path = buildPath(CanvasKit, requiredIndex(draw.paths, value.path, "clip path"), String(value.fillRule));
    try { canvas.clipPath(path, CanvasKit.ClipOp.Intersect, true); } finally { path.delete(); }
  } else fail("unsupported_clip", `clip '${String(clip.kind)}' is not closed`);
}

function buildPath(CanvasKit: CanvasKit, wire: { verbs: string[]; points: Array<[number, number]> }, fillRule: string): Path {
  const builder = new CanvasKit.PathBuilder();
  let point = 0;
  for (const verb of wire.verbs) {
    if (verb === "moveTo") builder.moveTo(...wire.points[point++]!);
    else if (verb === "lineTo") builder.lineTo(...wire.points[point++]!);
    else if (verb === "quadTo") { const a = wire.points[point++]!, b = wire.points[point++]!; builder.quadTo(a[0], a[1], b[0], b[1]); }
    else if (verb === "cubicTo") { const a = wire.points[point++]!, b = wire.points[point++]!, c = wire.points[point++]!; builder.cubicTo(a[0], a[1], b[0], b[1], c[0], c[1]); }
    else if (verb === "close") builder.close();
    else { builder.delete(); fail("path_verb", `unknown path verb '${verb}'`); }
  }
  builder.setFillType(fillRule === "evenOdd" ? CanvasKit.FillType.EvenOdd : CanvasKit.FillType.Winding);
  return builder.detachAndDelete();
}

function makePaint(CanvasKit: CanvasKit, raw: unknown): Paint {
  const wire = record(raw, "DrawProgram paint");
  const paint = new CanvasKit.Paint();
  paint.setAntiAlias(true);
  try {
    if (wire.kind === "solid") {
      paint.setColor(linearColor(CanvasKit, record(wire.value, "solid color") as unknown as LinearColorWire));
      return paint;
    }
    const value = record(wire.value, `${String(wire.kind)} paint`);
    const stops = array(value.stops, "gradient stops").map((rawStop) => record(rawStop, "gradient stop"));
    const colors = stops.map((stop) => linearColor(CanvasKit, record(stop.color, "gradient stop color") as unknown as LinearColorWire));
    const positions = stops.map((stop) => finiteNumber(stop.offset, "gradient stop offset"));
    const mode = tileMode(CanvasKit, String(value.spread));
    let shader: Shader | null = null;
    if (wire.kind === "linearGradient") {
      shader = CanvasKit.Shader.MakeLinearGradient(
        pair(value.start, "linear gradient start"),
        pair(value.end, "linear gradient end"),
        colors,
        positions,
        mode,
        undefined,
        1,
        CanvasKit.ColorSpace.SRGB,
      );
    } else if (wire.kind === "radialGradient") {
      const center = pair(value.center, "radial gradient center");
      const radii = pair(value.radii, "radial gradient radii");
      const radius = Math.max(radii[0], radii[1]);
      if (!(radius > 0)) fail("gradient", "radial gradient radius must be positive");
      shader = CanvasKit.Shader.MakeRadialGradient(
        center,
        radius,
        colors,
        positions,
        mode,
        CanvasKit.Matrix.scaled(radii[0] / radius, radii[1] / radius, center[0], center[1]),
        1,
        CanvasKit.ColorSpace.SRGB,
      );
    } else if (wire.kind === "conicGradient") {
      const center = pair(value.center, "conic gradient center");
      const start = finiteNumber(value.startAngleDegrees, "conic gradient start angle");
      shader = CanvasKit.Shader.MakeSweepGradient(
        center[0],
        center[1],
        colors,
        positions,
        mode,
        null,
        1,
        start,
        start + finiteNumber(value.sweepAngleDegrees, "conic gradient sweep angle"),
        CanvasKit.ColorSpace.SRGB,
      );
    } else {
      fail("unsupported_paint", `paint '${String(wire.kind)}' is not closed in CanvasKit`);
    }
    if (!shader) fail("gradient", `${String(wire.kind)} shader creation failed`);
    paint.setShader(shader);
    shader.delete();
    return paint;
  } catch (error) {
    paint.delete();
    throw error;
  }
}

function roundRect(CanvasKit: CanvasKit, wire: Wire): Float32Array {
  const bounds = rect(wire.rect, "round rect bounds");
  const radii = array(wire.radii, "round rect radii").map((raw) => pair(raw, "round rect radius"));
  if (radii.length !== 4) fail("round_rect", "round rect needs four radii");
  return Float32Array.from([
    bounds.x,
    bounds.y,
    bounds.x + bounds.width,
    bounds.y + bounds.height,
    radii[0]![0], radii[0]![1],
    radii[1]![0], radii[1]![1],
    radii[2]![0], radii[2]![1],
    radii[3]![0], radii[3]![1],
  ]);
}

function configureStrokePaint(CanvasKit: CanvasKit, paint: Paint, stroke: Wire) {
  paint.setStyle(CanvasKit.PaintStyle.Stroke);
  paint.setStrokeWidth(finiteNumber(stroke.width, "stroke width"));
  paint.setStrokeCap(stroke.cap === "round" ? CanvasKit.StrokeCap.Round : stroke.cap === "square" ? CanvasKit.StrokeCap.Square : CanvasKit.StrokeCap.Butt);
  paint.setStrokeJoin(stroke.join === "round" ? CanvasKit.StrokeJoin.Round : stroke.join === "bevel" ? CanvasKit.StrokeJoin.Bevel : CanvasKit.StrokeJoin.Miter);
  paint.setStrokeMiter(finiteNumber(stroke.miterLimit, "stroke miter"));
  const dash = array(stroke.dash, "stroke dash").map((value) => finiteNumber(value, "dash value"));
  const effect = dash.length ? CanvasKit.PathEffect.MakeDash(dash, finiteNumber(stroke.dashOffset, "dash offset")) : null;
  if (effect) paint.setPathEffect(effect);
  return effect;
}

function tileMode(CanvasKit: CanvasKit, spread: string) {
  if (spread === "pad") return CanvasKit.TileMode.Clamp;
  if (spread === "repeat") return CanvasKit.TileMode.Repeat;
  if (spread === "reflect") return CanvasKit.TileMode.Mirror;
  fail("gradient", `gradient spread '${spread}' is outside the closed set`);
}

function programMatrix(admitted: AdmittedProgram, localPlan: Wire, output: unknown, device: number[]): number[] {
  const id = positiveId(output, "program output resource");
  const resource = array(localPlan.resources, "localPlan.resources").map((value) => record(value, "local resource")).find((value) => value.id === id);
  if (!resource) fail("program_resource", `local resource ${id} is absent`);
  return programTransformMatrix(
    admitted,
    numberArray(resource.localToProgram, 9, "localToProgram"),
    device,
  );
}

function programTransformMatrix(admitted: AdmittedProgram, localToProgram: number[], device: number[]): number[] {
  const viewport = admitted.draw.viewport;
  if (!(viewport.width > 0) || !(viewport.height > 0)) fail("program_viewport", "DrawProgram viewport is empty during execution");
  const normalize = [1 / viewport.width, 0, -viewport.x / viewport.width, 0, 1 / viewport.height, -viewport.y / viewport.height, 0, 0, 1];
  return mul3(device, mul3(normalize, localToProgram));
}

function requireGlassKernel(kernel: CanvasKitGlassKernel | null): CanvasKitGlassKernel {
  if (!kernel) fail("motion_glass_kernel", "ProductEngine Motion Glass kernel is not attached");
  return kernel;
}

export function drawMotionGlassShader(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  value: ImageValue,
  outputRoi: DeviceRoi,
  uniforms: Float32Array,
): void {
  if (!value.image || value.roi.width <= 0 || value.roi.height <= 0) {
    fail("motion_glass_input", "Motion Glass requires a materialized non-empty ROI input");
  }
  if (!(uniforms instanceof Float32Array) || uniforms.length !== MOTION_GLASS_UNIFORM_FLOATS) {
    fail("motion_glass_uniforms", `kernel returned ${uniforms.length} floats, expected ${MOTION_GLASS_UNIFORM_FLOATS}`);
  }
  const child = imageValueShader(CanvasKit, value, "linearClamp");
  let shader: Shader | null = null;
  const paint = new CanvasKit.Paint();
  try {
    shader = builtins.shader("motionGlass", uniforms, [child]);
    paint.setBlendMode(CanvasKit.BlendMode.Src);
    paint.setShader(shader);
    canvas.drawRect(
      CanvasKit.XYWHRect(outputRoi.x, outputRoi.y, outputRoi.width, outputRoi.height),
      paint,
    );
  } finally {
    paint.delete();
    shader?.delete();
    child.delete();
  }
}

class SurfaceArena {
  private readonly slots = new Map<string, { surface: Surface; width: number; height: number }>();
  private readonly freeTransients = new Map<Surface, { width: number; height: number }>();
  private readonly checkedOutTransients = new Map<Surface, { width: number; height: number }>();
  private readonly rasterMemory = new Map<Surface, MallocObj>();
  private readonly surfaceBytes = new Map<Surface, bigint>();
  private readonly usedSlotKeys = new Set<string>();
  private outputSurface: Surface | null = null;
  private outputUsed = false;
  residentBytes = 0n;
  frameAllocations = 0;
  frameReuses = 0;
  frameAllocatedBytes = 0n;
  framePeakBytes = 0n;
  frameEvictions = 0;
  constructor(
    private readonly CanvasKit: CanvasKit,
    private readonly context: GrDirectContext | null,
    private readonly width: number,
    private readonly height: number,
    private readonly maxSurfaceBytes: bigint,
    private readonly maxFrameBytes: bigint,
  ) {}
  matches(
    context: GrDirectContext | null,
    width: number,
    height: number,
    maxSurfaceBytes: bigint,
    maxFrameBytes: bigint,
  ): boolean {
    return this.context === context
      && this.width === width
      && this.height === height
      && this.maxSurfaceBytes === maxSurfaceBytes
      && this.maxFrameBytes === maxFrameBytes;
  }
  beginFrame(): void {
    if (this.checkedOutTransients.size !== 0) fail("surface_lifetime", "transient surface escaped the prior frame");
    this.usedSlotKeys.clear();
    this.outputUsed = false;
    this.frameAllocations = 0;
    this.frameReuses = 0;
    this.frameAllocatedBytes = 0n;
    this.framePeakBytes = this.residentBytes;
    this.frameEvictions = 0;
  }
  endFrame(): void {
    const leaked = this.checkedOutTransients.size;
    if (leaked !== 0) {
      // Restore the arena before rejecting the frame so one failed draw cannot poison every later
      // seek. Snapshots own their pixels independently; checked-out surfaces remain arena-owned.
      for (const [surface, dimensions] of this.checkedOutTransients) {
        this.freeTransients.set(surface, dimensions);
      }
      this.checkedOutTransients.clear();
    }
    if (leaked !== 0) fail("surface_lifetime", `${leaked} transient surface(s) escaped frame execution`);
  }
  prepareReport(): void {
    if (this.checkedOutTransients.size !== 0) {
      fail("surface_lifetime", "transient surface escaped successful frame execution");
    }
  }
  get count(): number {
    return this.slots.size
      + this.freeTransients.size
      + this.checkedOutTransients.size
      + Number(this.outputSurface !== null);
  }
  slot(id: number, width: number, height: number): Surface {
    return this.keyedSlot(`outer:${id}`, width, height);
  }
  programSlot(program: number, id: number, width: number, height: number): Surface {
    return this.keyedSlot(`program:${program}:${id}`, width, height);
  }
  private keyedSlot(key: string, width: number, height: number): Surface {
    this.usedSlotKeys.add(key);
    const existing = this.slots.get(key);
    // A physical ROI occupies the top-left of its working surface. A larger allocation therefore
    // remains a valid backing store when animation makes the next frame's ROI smaller; the exact
    // logical bounds still travel with the snapshot and are cropped by every consumer.
    if (existing && existing.width >= width && existing.height >= height) {
      this.frameReuses += 1;
      return existing.surface;
    }
    if (existing) {
      this.slots.delete(key);
      this.evictSurface(existing.surface);
    }
    const surface = this.create(width, height);
    this.slots.set(key, { surface, width, height });
    return surface;
  }
  output(): Surface {
    this.outputUsed = true;
    if (this.outputSurface) { this.frameReuses += 1; return this.outputSurface; }
    return this.outputSurface = this.create(this.width, this.height);
  }
  transient(width = this.width, height = this.height): Surface {
    const reusable = [...this.freeTransients]
      .filter(([, dimensions]) => dimensions.width >= width && dimensions.height >= height)
      .sort((left, right) => left[1].width * left[1].height - right[1].width * right[1].height)[0];
    const surface = reusable ? reusable[0] : this.create(width, height);
    if (reusable) {
      this.freeTransients.delete(surface);
      this.frameReuses += 1;
    }
    this.checkedOutTransients.set(surface, { width, height });
    return surface;
  }
  releaseTransient(surface: Surface): void {
    const dimensions = this.checkedOutTransients.get(surface);
    if (!dimensions || !this.checkedOutTransients.delete(surface)) {
      fail("surface_lifetime", "transient surface was released twice or by the wrong arena");
    }
    this.freeTransients.set(surface, dimensions);
  }
  delete(): void {
    const all = new Set<Surface>([
      ...[...this.slots.values()].map((entry) => entry.surface),
      ...this.freeTransients.keys(),
      ...this.checkedOutTransients.keys(),
      ...(this.outputSurface ? [this.outputSurface] : []),
    ]);
    for (const surface of all) this.deleteSurface(surface);
    this.slots.clear();
    this.freeTransients.clear();
    this.checkedOutTransients.clear();
    this.outputSurface = null;
  }
  private create(width: number, height: number): Surface {
    const bytes = BigInt(width) * BigInt(height) * 8n;
    if (width <= 0 || height <= 0 || bytes <= 0n || bytes > this.maxSurfaceBytes) {
      fail("surface_budget", `${width}x${height} working surface needs ${bytes} bytes, budget ${this.maxSurfaceBytes}`);
    }
    this.ensureCapacity(bytes);
    const info = {
      width,
      height,
      colorType: this.CanvasKit.ColorType.RGBA_F16,
      alphaType: this.CanvasKit.AlphaType.Premul,
      colorSpace: this.CanvasKit.ColorSpace.SRGB,
    };
    let memory: MallocObj | null = null;
    let surface: Surface | null;
    if (this.context) {
      surface = this.CanvasKit.MakeRenderTarget(this.context, info);
    } else {
      const rowBytes = width * 8;
      const byteLength = rowBytes * height;
      if (!Number.isSafeInteger(byteLength) || byteLength <= 0) {
        fail("surface_allocation", "working F16 byte length is outside the exact integer range");
      }
      memory = this.CanvasKit.Malloc(Uint8Array, byteLength);
      surface = this.CanvasKit.MakeRasterDirectSurface(info, memory, rowBytes);
      if (!surface) this.CanvasKit.Free(memory);
    }
    if (!surface) fail("surface_allocation", `cannot allocate ${width}x${height} working surface`);
    if (memory) this.rasterMemory.set(surface, memory);
    this.surfaceBytes.set(surface, bytes);
    this.residentBytes += bytes;
    this.frameAllocations += 1;
    this.frameAllocatedBytes += bytes;
    this.framePeakBytes = this.residentBytes > this.framePeakBytes ? this.residentBytes : this.framePeakBytes;
    return surface;
  }
  private ensureCapacity(requiredBytes: bigint): void {
    while (this.residentBytes + requiredBytes > this.maxFrameBytes) {
      const free = this.freeTransients.entries().next();
      if (!free.done) {
        this.freeTransients.delete(free.value[0]);
        this.evictSurface(free.value[0]);
        continue;
      }
      const unused = [...this.slots].find(([key]) => !this.usedSlotKeys.has(key));
      if (unused) {
        this.slots.delete(unused[0]);
        this.evictSurface(unused[1].surface);
        continue;
      }
      if (!this.outputUsed && this.outputSurface) {
        const surface = this.outputSurface;
        this.outputSurface = null;
        this.evictSurface(surface);
        continue;
      }
      fail(
        "frame_budget",
        `working surface pool needs ${this.residentBytes + requiredBytes} bytes, budget ${this.maxFrameBytes}`,
      );
    }
  }
  private evictSurface(surface: Surface): void {
    this.frameEvictions += 1;
    this.deleteSurface(surface);
  }
  private deleteSurface(surface: Surface): void {
    const bytes = this.surfaceBytes.get(surface) ?? 0n;
    surface.delete();
    const memory = this.rasterMemory.get(surface);
    if (memory) {
      this.CanvasKit.Free(memory);
      this.rasterMemory.delete(surface);
    }
    this.surfaceBytes.delete(surface);
    this.residentBytes -= bytes;
  }
}

interface BoundOuterSchedule {
  readonly resourceRois: Map<number, DeviceRoi>;
  readonly slotExtents: Map<number, { width: number; height: number } | null>;
  readonly slotByResource: Map<number, number>;
  readonly programs: Map<number, BoundProgramRuntimeSchedule>;
}

function admitBoundOuterSchedule(
  plan: Wire,
  programs: Wire[],
  schedule: Wire,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
): BoundOuterSchedule {
  const root: DeviceRoi = { x: 0, y: 0, ...extent };
  const resourceRois = new Map<number, DeviceRoi>();
  const planResources = array(plan.resources, "plan.resources");
  const boundResources = array(schedule.resources, "schedule.resources");
  if (planResources.length !== boundResources.length) fail("bound_schedule", "resource count mismatch");
  for (let index = 0; index < planResources.length; index += 1) {
    const resource = record(planResources[index], `plan.resources[${index}]`);
    const id = positiveId(resource.id, `plan.resources[${index}].id`);
    const roiWire = record(resource.roi, `plan.resources[${index}].roi`);
    const expected = roiWire.kind === "fullFrame"
      ? root
      : roiWire.kind === "static"
        ? deviceRoi(record(roiWire.rect, "static ROI"), extent)
        : roiWire.kind === "dynamic"
          ? deviceRoi(dynamicBounds(dynamic, roiWire.binding), extent)
          : fail("bound_schedule", `unknown ROI kind '${String(roiWire.kind)}'`);
    const actual = record(boundResources[index], `schedule.resources[${index}]`);
    if (positiveId(actual.resource, "bound resource id") !== id
      || !sameRoi(deviceRoi(record(actual.deviceRoi, "bound device ROI"), extent), expected)) {
      fail("bound_schedule", `resource ${id} ROI is not the exact plan/binding projection`);
    }
    resourceRois.set(id, expected);
  }

  const slotExtents = new Map<number, { width: number; height: number } | null>();
  const slotByResource = new Map<number, number>();
  const planSlots = array(plan.surfaceSlots, "plan.surfaceSlots");
  const boundSlots = array(schedule.surfaceSlots, "schedule.surfaceSlots");
  if (planSlots.length !== boundSlots.length) fail("bound_schedule", "surface slot count mismatch");
  const activeBytes = Array.from({ length: array(plan.passes, "plan.passes").length }, () => 0n);
  for (let index = 0; index < planSlots.length; index += 1) {
    const slot = record(planSlots[index], `plan.surfaceSlots[${index}]`);
    const bound = record(boundSlots[index], `schedule.surfaceSlots[${index}]`);
    const id = positiveId(slot.id, "surface slot id");
    if (positiveId(bound.id, "bound surface slot id") !== id) fail("bound_schedule", "surface slot IDs diverge");
    const allocations = array(slot.allocations, "surface allocations");
    const boundAllocations = array(bound.allocations, "bound surface allocations");
    if (allocations.length !== boundAllocations.length) fail("bound_schedule", `slot ${id} allocation count mismatch`);
    let width = 0;
    let height = 0;
    for (let allocationIndex = 0; allocationIndex < allocations.length; allocationIndex += 1) {
      const allocation = record(allocations[allocationIndex], "surface allocation");
      const actual = record(boundAllocations[allocationIndex], "bound surface allocation");
      const resource = positiveId(allocation.resource, "surface allocation resource");
      const roi = required(resourceRois, resource, "bound resource ROI");
      if (positiveId(actual.resource, "bound allocation resource") !== resource
        || String(actual.reason) !== String(allocation.reason)
        || !deepEqual(actual.interval, allocation.interval)
        || !sameRoi(deviceRoi(record(actual.deviceRoi, "bound allocation ROI"), extent), roi)) {
        fail("bound_schedule", `slot ${id} allocation ${allocationIndex} diverges from the plan`);
      }
      const regional = allocation.reason === "layerIntermediate"
        || allocation.reason === "backdropResolve"
        || allocation.reason === "programIntermediate";
      width = Math.max(width, regional ? roi.width : extent.width);
      height = Math.max(height, regional ? roi.height : extent.height);
      slotByResource.set(resource, id);
    }
    const expectedExtent = width === 0 || height === 0 ? null : { width, height };
    const actualExtent = bound.extent == null
      ? null
      : {
          width: positiveId(record(bound.extent, "bound slot extent").width, "bound slot width"),
          height: positiveId(record(bound.extent, "bound slot extent").height, "bound slot height"),
        };
    if (!deepEqual(actualExtent, expectedExtent)) fail("bound_schedule", `slot ${id} extent is not derived from its allocations`);
    const bytes = BigInt(width) * BigInt(height) * 8n;
    if (BigInt(positiveIdOrZero(bound.estimatedBytes, "bound slot bytes")) !== bytes) {
      fail("bound_schedule", `slot ${id} byte estimate mismatch`);
    }
    for (const rawAllocation of allocations) {
      const interval = record(record(rawAllocation, "surface allocation").interval, "surface interval");
      const first = positiveId(interval.first, "surface interval first");
      const last = positiveId(interval.last, "surface interval last");
      for (let pass = first; pass <= last; pass += 1) activeBytes[pass - 1] = (activeBytes[pass - 1] ?? 0n) + bytes;
    }
    slotExtents.set(id, expectedExtent);
  }
  const outerPeak = activeBytes.reduce((peak, value) => value > peak ? value : peak, 0n);
  if (BigInt(positiveIdOrZero(schedule.outerPeakSurfaceBytes, "outer peak bytes")) !== outerPeak) {
    fail("bound_schedule", "outer peak byte estimate mismatch");
  }
  const boundPrograms = admitBoundProgramSchedules(plan, programs, schedule, dynamic, extent, activeBytes);
  const combinedPeak = activeBytes.reduce((peak, value) => value > peak ? value : peak, 0n);
  if (BigInt(positiveIdOrZero(schedule.estimatedPeakSurfaceBytes, "combined peak bytes")) !== combinedPeak) {
    fail("bound_schedule", "combined plan/program peak byte estimate mismatch");
  }
  return { resourceRois, slotExtents, slotByResource, programs: boundPrograms };
}

function admitBoundProgramSchedules(
  plan: Wire,
  framePrograms: Wire[],
  schedule: Wire,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
  activeBytes: bigint[],
): Map<number, BoundProgramRuntimeSchedule> {
  const result = new Map<number, BoundProgramRuntimeSchedule>();
  const programLayouts = array(plan.programs, "plan.programs");
  const boundPrograms = array(schedule.programs, "schedule.programs");
  if (programLayouts.length !== framePrograms.length || framePrograms.length !== boundPrograms.length) {
    fail("program_schedule", "bound program count does not match the plan");
  }

  const executionByProgram = new Map<number, { pass: number; kind: Wire }>();
  for (const rawPass of array(plan.passes, "plan.passes")) {
    const pass = record(rawPass, "plan pass");
    const kind = record(pass.kind, "plan pass kind");
    if (kind.kind !== "rasterProgram" && kind.kind !== "rasterCaption") continue;
    const program = positiveId(kind.program, "program execution id");
    if (executionByProgram.has(program)) fail("program_schedule", `program ${program} has more than one execution pass`);
    executionByProgram.set(program, { pass: positiveId(pass.id, "program execution pass"), kind });
  }

  for (let index = 0; index < framePrograms.length; index += 1) {
    const layout = record(programLayouts[index], `plan.programs[${index}]`);
    const program = record(framePrograms[index], `bindings.programs[${index}]`);
    const bound = record(boundPrograms[index], `schedule.programs[${index}]`);
    const id = positiveId(program.id, `bindings.programs[${index}].id`);
    if (id !== index + 1
      || positiveId(layout.id, `plan.programs[${index}].id`) !== id
      || positiveId(bound.program, "bound program id") !== id) {
      fail("program_schedule", "program IDs are not canonical and aligned");
    }
    const execution = required(executionByProgram, id, "program execution pass");
    if (positiveId(bound.executionPass, "bound program execution pass") !== execution.pass
      || String(bound.boundsReason) !== String(execution.kind.boundsReason)) {
      fail("program_schedule", `program ${id} execution identity diverges from the plan`);
    }

    const localPlan = record(program.localPlan, `program ${id} localPlan`);
    const localSchedule = record(program.localSchedule, `program ${id} localSchedule`);
    const storageByResource = new Map<number, Wire>();
    for (const rawStorage of array(localSchedule.resources, "program schedule resources")) {
      const storage = record(rawStorage, "program resource storage");
      const resource = positiveId(storage.resource, "program storage resource");
      if (storageByResource.has(resource)) fail("program_schedule", `program ${id} repeats resource ${resource}`);
      storageByResource.set(resource, record(storage.storage, "program storage kind"));
    }

    const destinationRois = new Map<number, DeviceRoi>();
    for (const rawPass of array(localPlan.passes, "program local passes")) {
      const localKind = record(record(rawPass, "program local pass").kind, "program local pass kind");
      if (localKind.kind !== "readDestination" || localKind.external == null) continue;
      const destination = positiveId(localKind.external, "program destination id");
      const use = record(requiredIndex(array(program.destinationUses, "program destination uses"), destination - 1, "program destination use"), "program destination use");
      const roi = deviceRoi(dynamicBounds(dynamic, use.sampleBounds), extent);
      const output = positiveId(localKind.output, "program destination output");
      const prior = destinationRois.get(output);
      if (prior && !sameRoi(prior, roi)) fail("program_schedule", `program ${id} destination ROI has conflicting derivations`);
      destinationRois.set(output, roi);
    }

    const device = dynamicTransform(dynamic, execution.kind.transform);
    const resourceRois = new Map<number, DeviceRoi>();
    const localResources = array(localPlan.resources, "program local resources");
    const boundResources = array(bound.resources, "bound program resources");
    if (localResources.length !== boundResources.length || storageByResource.size !== localResources.length) {
      fail("program_schedule", `program ${id} resource count mismatch`);
    }
    for (let resourceIndex = 0; resourceIndex < localResources.length; resourceIndex += 1) {
      const resource = record(localResources[resourceIndex], `program ${id} resource ${resourceIndex}`);
      const resourceId = positiveId(resource.id, "program resource id");
      if (resourceId !== resourceIndex + 1) fail("program_schedule", `program ${id} resource IDs are not canonical`);
      const storage = required(storageByResource, resourceId, "program storage");
      const expected = storage.kind === "transparent"
        ? emptyRoi()
        : String(execution.kind.boundsReason) === "conservativeCameraTarget"
          ? { x: 0, y: 0, ...extent }
          : destinationRois.get(resourceId)
            ?? projectProgramResource(program, resource, device, extent);
      const actual = record(boundResources[resourceIndex], "bound program resource");
      if (positiveId(actual.resource, "bound program resource id") !== resourceId
        || !sameRoi(deviceRoi(record(actual.deviceRoi, "bound program device ROI"), extent), expected)) {
        fail("program_schedule", `program ${id} resource ${resourceId} ROI is not the exact plan/binding projection`);
      }
      resourceRois.set(resourceId, expected);
    }

    const slotExtents = new Map<number, { width: number; height: number } | null>();
    const slotByResource = new Map<number, number>();
    const localSlots = array(localSchedule.surfaceSlots, "program surface slots");
    const boundSlots = array(bound.surfaceSlots, "bound program surface slots");
    if (localSlots.length !== boundSlots.length) fail("program_schedule", `program ${id} surface slot count mismatch`);
    let estimatedSurfaceBytes = 0n;
    for (let slotIndex = 0; slotIndex < localSlots.length; slotIndex += 1) {
      const slot = record(localSlots[slotIndex], "program surface slot");
      const boundSlot = record(boundSlots[slotIndex], "bound program surface slot");
      const slotId = positiveId(slot.id, "program surface slot id");
      if (slotId !== slotIndex + 1 || positiveId(boundSlot.id, "bound program slot id") !== slotId) {
        fail("program_schedule", `program ${id} surface slot IDs diverge`);
      }
      const allocations = array(slot.allocations, "program slot allocations");
      const boundAllocations = array(boundSlot.allocations, "bound program slot allocations");
      if (allocations.length !== boundAllocations.length) fail("program_schedule", `program ${id} slot ${slotId} allocation count mismatch`);
      let width = 0;
      let height = 0;
      for (let allocationIndex = 0; allocationIndex < allocations.length; allocationIndex += 1) {
        const allocation = record(allocations[allocationIndex], "program surface allocation");
        const actual = record(boundAllocations[allocationIndex], "bound program surface allocation");
        const resource = positiveId(allocation.resource, "program allocation resource");
        const roi = required(resourceRois, resource, "program allocation ROI");
        if (positiveId(actual.resource, "bound program allocation resource") !== resource
          || String(actual.reason) !== String(allocation.reason)
          || !deepEqual(actual.interval, allocation.interval)
          || !sameRoi(deviceRoi(record(actual.deviceRoi, "bound program allocation ROI"), extent), roi)) {
          fail("program_schedule", `program ${id} slot ${slotId} allocation ${allocationIndex} diverges`);
        }
        if (slotByResource.has(resource)) {
          fail("program_schedule", `program ${id} resource ${resource} has more than one physical slot`);
        }
        slotByResource.set(resource, slotId);
        width = Math.max(width, roi.width);
        height = Math.max(height, roi.height);
      }
      const expectedExtent = width === 0 || height === 0 ? null : { width, height };
      const actualExtent = boundSlot.extent == null
        ? null
        : {
            width: positiveId(record(boundSlot.extent, "bound program extent").width, "bound program width"),
            height: positiveId(record(boundSlot.extent, "bound program extent").height, "bound program height"),
          };
      if (!deepEqual(actualExtent, expectedExtent)) fail("program_schedule", `program ${id} slot ${slotId} extent mismatch`);
      const bytes = BigInt(width) * BigInt(height) * 8n;
      if (BigInt(positiveIdOrZero(boundSlot.estimatedBytes, "bound program slot bytes")) !== bytes) {
        fail("program_schedule", `program ${id} slot ${slotId} byte estimate mismatch`);
      }
      estimatedSurfaceBytes += bytes;
      slotExtents.set(slotId, expectedExtent);
    }
    if (BigInt(positiveIdOrZero(bound.estimatedSurfaceBytes, "bound program bytes")) !== estimatedSurfaceBytes) {
      fail("program_schedule", `program ${id} total surface byte estimate mismatch`);
    }
    const active = activeBytes[execution.pass - 1];
    if (active === undefined) fail("program_schedule", `program ${id} execution pass is out of range`);
    activeBytes[execution.pass - 1] = active + estimatedSurfaceBytes;
    result.set(id, { resourceRois, slotExtents, slotByResource, estimatedSurfaceBytes });
  }

  if (executionByProgram.size !== result.size) {
    fail("program_schedule", "every program must have exactly one bound execution schedule");
  }
  return result;
}

function projectProgramResource(
  program: Wire,
  resource: Wire,
  device: number[],
  extent: { width: number; height: number },
): DeviceRoi {
  const bounds = record(resource.bounds, "program resource bounds");
  if (bounds.kind === "empty") return emptyRoi();
  if (bounds.kind !== "finite") fail("program_schedule", `unknown local bounds kind '${String(bounds.kind)}'`);
  return projectProgramResourceRoi(
    rect(program.viewport, "program viewport"),
    rect(bounds.rect, "program finite bounds"),
    numberArray(resource.localToProgram, 9, "program localToProgram"),
    device,
    extent,
  );
}

/**
 * Mirror Rust's conservative two-stage program projection exactly.
 *
 * A resource is first mapped through `localToProgram` and collapsed to a program-space AABB;
 * only that AABB is normalized through the program viewport and projected into device space.
 * Composing both matrices and projecting the original four corners is geometrically tighter but
 * is not the same storage proof when the two transforms contain opposing shear/rotation.
 */
export function projectProgramResourceRoi(
  viewport: RectWire,
  value: RectWire,
  localToProgram: number[],
  device: number[],
  extent: { width: number; height: number },
): DeviceRoi {
  if (viewport.width <= 0 || viewport.height <= 0) fail("program_schedule", "program viewport is empty");
  if (value.width <= 0 || value.height <= 0) return emptyRoi();
  const localCorners: Array<[number, number]> = [
    [value.x, value.y],
    [value.x + value.width, value.y],
    [value.x + value.width, value.y + value.height],
    [value.x, value.y + value.height],
  ];
  validateProjectionDomain(localToProgram, localCorners, Number.EPSILON, "program local resource");
  const localProjected = localCorners.map((corner) => projectValidated(localToProgram, corner));
  const mapped = {
    left: Math.min(...localProjected.map((point) => point[0])),
    top: Math.min(...localProjected.map((point) => point[1])),
    right: Math.max(...localProjected.map((point) => point[0])),
    bottom: Math.max(...localProjected.map((point) => point[1])),
  };
  const corners: Array<[number, number]> = [
    [(mapped.left - viewport.x) / viewport.width, (mapped.top - viewport.y) / viewport.height],
    [(mapped.right - viewport.x) / viewport.width, (mapped.top - viewport.y) / viewport.height],
    [(mapped.left - viewport.x) / viewport.width, (mapped.bottom - viewport.y) / viewport.height],
    [(mapped.right - viewport.x) / viewport.width, (mapped.bottom - viewport.y) / viewport.height],
  ];
  validateProjectionDomain(device, corners, 1e-12, "program device resource");
  const points = corners.map((corner) => projectValidated(device, corner));
  const left = Math.max(0, Math.min(extent.width, Math.floor(Math.min(...points.map((point) => point[0])))));
  const top = Math.max(0, Math.min(extent.height, Math.floor(Math.min(...points.map((point) => point[1])))));
  const right = Math.max(0, Math.min(extent.width, Math.ceil(Math.max(...points.map((point) => point[0])))));
  const bottom = Math.max(0, Math.min(extent.height, Math.ceil(Math.max(...points.map((point) => point[1])))));
  return right <= left || bottom <= top
    ? emptyRoi()
    : { x: left, y: top, width: right - left, height: bottom - top };
}

function validateProjectionDomain(
  matrix: number[],
  corners: Array<[number, number]>,
  relativeTolerance: number,
  label: string,
): void {
  const denominators = corners.map(([x, y]) => matrix[6]! * x + matrix[7]! * y + matrix[8]!);
  const maximum = denominators.reduce((current, value) => Math.max(current, Math.abs(value)), 0);
  // Transform2d::map_bounds uses an absolute f64::EPSILON threshold. DeviceTransform uses a
  // scale-relative 1e-12 horizon threshold. The caller selects the matching policy.
  const tolerance = relativeTolerance === Number.EPSILON
    ? Number.EPSILON
    : maximum * relativeTolerance;
  if (!Number.isFinite(maximum) || maximum === 0
    || denominators.some((value) => !Number.isFinite(value) || Math.abs(value) <= tolerance)
    || denominators.some((value) => Math.sign(value) !== Math.sign(denominators[0]!))) {
    fail("program_schedule", `${label} crosses the perspective horizon`);
  }
}

function projectValidated(matrix: number[], point: [number, number]): [number, number] {
  const w = matrix[6]! * point[0] + matrix[7]! * point[1] + matrix[8]!;
  const projected: [number, number] = [
    (matrix[0]! * point[0] + matrix[1]! * point[1] + matrix[2]!) / w,
    (matrix[3]! * point[0] + matrix[4]! * point[1] + matrix[5]!) / w,
  ];
  if (!projected.every(Number.isFinite)) fail("program_schedule", "program resource projection is invalid");
  return projected;
}

function emptyRoi(): DeviceRoi {
  return { x: 0, y: 0, width: 0, height: 0 };
}

function deviceRoi(value: Wire, extent: { width: number; height: number }): DeviceRoi {
  const roi = {
    x: positiveIdOrZero(value.x, "ROI x"),
    y: positiveIdOrZero(value.y, "ROI y"),
    width: positiveIdOrZero(value.width, "ROI width"),
    height: positiveIdOrZero(value.height, "ROI height"),
  };
  if (roi.x + roi.width > extent.width || roi.y + roi.height > extent.height) {
    fail("bound_schedule", "ROI escapes the render root");
  }
  if ((roi.width === 0 || roi.height === 0) && !sameRoi(roi, { x: 0, y: 0, width: 0, height: 0 })) {
    fail("bound_schedule", "empty ROI is not canonical");
  }
  return roi;
}

function sameRoi(left: DeviceRoi, right: DeviceRoi): boolean {
  return left.x === right.x && left.y === right.y
    && left.width === right.width && left.height === right.height;
}

function retireOuterResources(plan: Wire, passId: number, values: Map<number, ImageValue>, except: number): void {
  for (const rawSlot of array(plan.surfaceSlots, "surfaceSlots")) {
    for (const rawAllocation of array(record(rawSlot, "surface slot").allocations, "surface allocations")) {
      const allocation = record(rawAllocation, "surface allocation");
      if (positiveId(record(allocation.interval, "allocation interval").last, "allocation last") !== passId) continue;
      const id = positiveId(allocation.resource, "allocation resource");
      if (id === except) continue;
      const value = values.get(id);
      if (value?.owned) value.image?.delete();
      values.delete(id);
    }
  }
}

function disposeValues(values: Map<number, ImageValue>, retained: Image | null): void {
  const deleted = new Set<Image>();
  for (const value of values.values()) if (value.owned && value.image && value.image !== retained && !deleted.has(value.image)) { value.image.delete(); deleted.add(value.image); }
  if (retained && !deleted.has(retained)) retained.delete();
  values.clear();
}

function disposePrograms(programs: Map<number, AdmittedProgram>): void {
  // Backend font/shader objects are executor-owned caches. Per-frame maps only borrow them.
  programs.clear();
}

function requiredBuiltinKernels(plan: Wire, programs: Wire[]): Set<BuiltinKernel> {
  const kernels = new Set<BuiltinKernel>();
  for (const rawPass of array(plan.passes, "plan.passes")) {
    const kind = record(record(rawPass, "plan pass").kind, "plan pass kind");
    if (kind.kind === "dispatchKernel") {
      const invocation = record(kind.invocation, "kernel invocation");
      if (invocation.kind === "transition") {
        kernels.add(transitionKernel(String(invocation.kernel)));
        kernels.add("opacity");
      } else if (invocation.kind === "filter" || invocation.kind === "adjustmentEffect") {
        const effect = record(record(invocation.effect, "effect").kernel, "effect kernel");
        const required = effectBuiltin(String(effect.kind));
        if (required) kernels.add(required);
      }
    } else if (kind.kind === "importRegion") {
      const pipeline = record(kind.sourcePipeline, "source pipeline");
      if (pipeline.chromaKey != null) kernels.add("chromaKey");
    } else if (kind.kind === "compositeRegion") {
      const mode = record(kind.mode, "composite mode");
      if (mode.kind === "blend" && String(mode.mode) !== "normal") kernels.add("creativeBlend");
    } else if (kind.kind === "copyConvert") {
      kernels.add("outputTransform");
    }
  }
  for (const rawProgram of programs) {
    const localPlan = record(record(rawProgram, "binding program").localPlan, "local plan");
    for (const rawPass of array(localPlan.passes, "local plan passes")) {
      const kind = record(record(rawPass, "local pass").kind, "local pass kind");
      if (kind.kind === "blend" && String(kind.mode) !== "normal") kernels.add("creativeBlend");
      if (kind.kind === "motionGlass" || kind.kind === "applyMotionGlassForeground") {
        kernels.add("motionGlass");
      }
    }
  }
  return kernels;
}

function admitCanvasKitOutput(plan: Wire): void {
  const output = record(record(plan.renderSpec, "render spec").output, "output spec");
  if (output.bitDepth !== "eight") {
    fail("output_storage", `CanvasKit product targets accept eight-bit delivery, not '${String(output.bitDepth)}'`);
  }
  const dither = record(output.dither, "output dither");
  if (dither.kind !== "none") {
    fail("output_dither", "CanvasKit cannot reproduce the compositor's exact 64-bit triangular dither ABI");
  }
}

function drawOutputTransform(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  input: Image | null,
  operation: Wire,
): void {
  if (operation.kind !== "outputTransform") {
    fail("copy_operation", `copy operation '${String(operation.kind)}' is outside the closed set`);
  }
  const spec = record(operation.spec, "output transform spec");
  const target = record(spec.target, "output color target");
  const luminance = record(spec.luminance, "output luminance");
  drawBuiltinImage(
    CanvasKit,
    builtins,
    canvas,
    "outputTransform",
    [
      primariesCode(String(target.primaries)),
      transferCode(String(target.transfer)),
      enumCode(String(spec.toneMap), ["none", "reinhardLuminance"], "tone map"),
      enumCode(String(spec.gamutMap), ["clip", "chromaCompress"], "gamut map"),
      enumCode(String(spec.alpha), ["opaque", "straightCoverage", "premultipliedCoverage"], "output alpha"),
      positiveId(luminance.referenceWhite, "output reference white"),
      positiveId(luminance.peak, "output peak luminance"),
    ],
    [input],
    "src",
  );
}

function enumCode(value: string, values: readonly string[], label: string): number {
  const index = values.indexOf(value);
  if (index < 0) fail("wire_enum", `${label} '${value}' is outside the closed set`);
  return index;
}

function transitionKernel(value: string): BuiltinKernel {
  if (!TRANSITION_KERNELS.has(value as BuiltinKernel)) {
    fail("transition_kernel", `transition kernel '${value}' is outside the closed set`);
  }
  return value as BuiltinKernel;
}

function effectBuiltin(kind: string): BuiltinKernel | null {
  switch (kind) {
    case "chromaKey": return "chromaKey";
    case "colorGrade": return "colorGrade";
    case "mosaic": return "mosaic";
    case "directionalBlur": return "directionalBlur";
    case "spotlight": return "spotlight";
    case "gaussianBlur": return null;
    default: fail("effect_kernel", `effect kernel '${kind}' is outside the closed set`);
  }
}

function drawBuiltinImage(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  kernel: BuiltinKernel,
  uniforms: readonly number[],
  images: readonly (Image | null)[],
  mode: string,
): void {
  const children = images.map((value) => imageOrTransparentShader(CanvasKit, value));
  try {
    const shader = builtins.shader(kernel, uniforms, children);
    try { drawShader(CanvasKit, canvas, shader, mode); }
    finally { shader.delete(); }
  } finally {
    for (const child of children) child.delete();
  }
}

function opacityShader(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  image: Image | null,
  opacity: number,
): Shader {
  const child = imageOrTransparentShader(CanvasKit, image);
  try { return builtins.shader("opacity", [opacity], [child]); }
  finally { child.delete(); }
}

function imageOrTransparentShader(CanvasKit: CanvasKit, image: Image | null): Shader {
  return image
    ? imageShader(CanvasKit, image, "linearClamp")
    : CanvasKit.Shader.MakeColor(CanvasKit.Color4f(0, 0, 0, 0), CanvasKit.ColorSpace.SRGB);
}

function imageValueOrTransparentShader(CanvasKit: CanvasKit, value: ImageValue): Shader {
  return value.image
    ? imageValueShader(CanvasKit, value, "linearClamp")
    : CanvasKit.Shader.MakeColor(CanvasKit.Color4f(0, 0, 0, 0), CanvasKit.ColorSpace.SRGB);
}

function drawShader(CanvasKit: CanvasKit, canvas: Canvas, shader: Shader, mode: string): void {
  const paint = new CanvasKit.Paint();
  try {
    paint.setShader(shader);
    paint.setBlendMode(blendMode(CanvasKit, mode));
    canvas.drawPaint(paint);
  } finally { paint.delete(); }
}

function drawCreativeBlend(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  source: Image | null,
  destination: Image | null,
  mode: string,
  opacity: number,
  compositeResult: boolean,
): void {
  if (!source) {
    if (compositeResult) drawImage(CanvasKit, canvas, destination, 1, "src");
    return;
  }
  const children = [
    imageOrTransparentShader(CanvasKit, source),
    imageOrTransparentShader(CanvasKit, destination),
  ];
  try {
    const shader = builtins.shader(
      "creativeBlend",
      [blendCode(mode), opacity, compositeResult ? 1 : 0],
      children,
    );
    try { drawShader(CanvasKit, canvas, shader, "src"); }
    finally { shader.delete(); }
  } finally {
    for (const child of children) child.delete();
  }
}

function drawCreativeBlendValues(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  source: ImageValue,
  destination: ImageValue,
  mode: string,
  opacity: number,
  compositeResult: boolean,
): void {
  if (!source.image) {
    if (compositeResult) drawImageValue(CanvasKit, canvas, destination, 1, "src");
    return;
  }
  const children = [
    imageValueOrTransparentShader(CanvasKit, source),
    imageValueOrTransparentShader(CanvasKit, destination),
  ];
  try {
    const shader = builtins.shader(
      "creativeBlend",
      [blendCode(mode), opacity, compositeResult ? 1 : 0],
      children,
    );
    try {
      drawShader(CanvasKit, canvas, shader, "src");
    } finally {
      shader.delete();
    }
  } finally {
    for (const child of children) child.delete();
  }
}

function blendCode(mode: string): number {
  const modes: Record<string, number> = {
    normal: 0,
    multiply: 1,
    screen: 2,
    overlay: 3,
    darken: 4,
    lighten: 5,
    colorDodge: 6,
    colorBurn: 7,
    linearBurn: 8,
    hardLight: 9,
    softLight: 10,
    difference: 11,
    exclusion: 12,
    hue: 13,
    saturation: 14,
    color: 15,
    luminosity: 16,
  };
  const code = modes[mode];
  if (code === undefined) fail("blend_mode", `blend mode '${mode}' is outside the closed set`);
  return code;
}

async function packedHash(packet: ArrayBuffer | ArrayBufferView): Promise<string> {
  const bytes = packet instanceof ArrayBuffer ? new Uint8Array(packet) : new Uint8Array(packet.buffer, packet.byteOffset, packet.byteLength);
  return `sha256:${await sha256Hex(bytes)}`;
}

function validatePacketTriple(
  plan: Wire,
  bindings: Wire,
  schedule: Wire,
  templateHash: string,
  bindingHash: string,
): void {
  if (schedule.templateHash !== templateHash || schedule.bindingHash !== bindingHash) {
    fail("schedule_hash", "bound schedule does not name the supplied plan/binding pair");
  }
  if (schedule.capabilityFingerprint !== plan.capabilityFingerprint) {
    fail("schedule_capability", "bound schedule capability fingerprint does not match the plan");
  }
  admitOptimizationReport(plan);
}

function exactPacketBytes(input: ArrayBuffer | ArrayBufferView): Uint8Array {
  if (input instanceof ArrayBuffer) return new Uint8Array(input);
  return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
}

/**
 * Admit the optimizer proof at the Web execution boundary.
 *
 * Rust already validates and hashes the normalized final state before packing. Web additionally
 * verifies that every reason-coded physical alias is named exactly once and points at the final
 * resource/pass pair it claims. This keeps a checksum-consistent but structurally forged packet
 * from silently turning optimizer evidence into an unreported execution shortcut.
 */
export function admitOptimizationReport(plan: Wire): void {
  const report = record(plan.optimization, "plan.optimization");
  const inputHash = digestString(report.inputPhysicalHash, "optimization.inputPhysicalHash");
  const outputHash = digestString(report.outputPhysicalHash, "optimization.outputPhysicalHash");
  const resources = array(plan.resources, "plan.resources").map((value, index) =>
    record(value, `plan.resources[${index}]`));
  const passes = array(plan.passes, "plan.passes").map((value, index) =>
    record(value, `plan.passes[${index}]`));
  const rewrites = array(report.rewrites, "optimization.rewrites");
  if (rewrites.length === 0) {
    if (inputHash !== outputHash) {
      fail("optimization_proof", "an identity optimization must preserve its physical hash");
    }
  } else if (inputHash === outputHash) {
    fail("optimization_proof", "a non-empty rewrite set must change the normalized physical state");
  }

  let previousPass = 0;
  const outputs = new Set<number>();
  const rewritePasses = new Set<number>();
  for (const [index, raw] of rewrites.entries()) {
    const rewrite = record(raw, `optimization.rewrites[${index}]`);
    const passId = positiveId(rewrite.pass, `optimization.rewrites[${index}].pass`);
    const outputId = positiveId(rewrite.output, `optimization.rewrites[${index}].output`);
    const sourceId = positiveId(rewrite.source, `optimization.rewrites[${index}].source`);
    if (passId <= previousPass || rewritePasses.has(passId)) {
      fail("optimization_proof", "rewrites must be ordered by unique execution pass");
    }
    previousPass = passId;
    rewritePasses.add(passId);
    if (outputs.has(outputId) || sourceId >= outputId) {
      fail("optimization_proof", "rewrite source/output identity is not canonical");
    }
    outputs.add(outputId);

    const resource = record(resources[outputId - 1], `plan.resources[${outputId - 1}]`);
    const pass = record(passes[passId - 1], `plan.passes[${passId - 1}]`);
    if (positiveId(resource.id, "optimization resource id") !== outputId
      || positiveId(pass.id, "optimization pass id") !== passId) {
      fail("optimization_proof", "rewrite resource/pass ids are not canonical");
    }
    const resourceKind = record(resource.kind, "optimization resource kind");
    const passKind = record(pass.kind, "optimization pass kind");
    const expected = rewrite.kind === "backdropResolveReuse"
      ? { resourceReason: "reusedBackdropResolve", passKind: "bindBackdropView" }
      : rewrite.kind === "noOpGroupElimination"
        ? { resourceReason: "noOpGroup", passKind: "aliasResource" }
        : null;
    if (!expected
      || resourceKind.kind !== "alias"
      || resourceKind.reason !== expected.resourceReason
      || positiveId(resourceKind.source, "optimization alias source") !== sourceId
      || passKind.kind !== expected.passKind
      || passKind.reason !== expected.resourceReason
      || positiveId(passKind.input, "optimization pass input") !== sourceId
      || positiveId(passKind.output, "optimization pass output") !== outputId) {
      fail("optimization_proof", `rewrite ${index} does not match the final plan`);
    }
  }

  for (const resource of resources) {
    const kind = record(resource.kind, "plan resource kind");
    if (kind.kind === "alias"
      && (kind.reason === "reusedBackdropResolve" || kind.reason === "noOpGroup")
      && !outputs.has(positiveId(resource.id, "optimizer alias resource id"))) {
      fail("optimization_proof", "an optimizer-owned resource alias is missing from the rewrite proof");
    }
  }
  for (const pass of passes) {
    const kind = record(pass.kind, "plan pass kind");
    const optimizerOwned = (kind.kind === "bindBackdropView" && kind.reason === "reusedBackdropResolve")
      || (kind.kind === "aliasResource" && kind.reason === "noOpGroup");
    if (optimizerOwned && !rewritePasses.has(positiveId(pass.id, "optimizer alias pass id"))) {
      fail("optimization_proof", "an optimizer-owned execution alias is missing from the rewrite proof");
    }
  }
}

function digestString(value: unknown, label: string): string {
  if (typeof value !== "string" || !/^sha256:[0-9a-f]{64}$/.test(value)) {
    fail("optimization_proof", `${label} must be a sha256:<64 lowercase hex> ContentDigest`);
  }
  return value;
}

function dynamicBindings(bindings: Wire): Map<number, Wire> {
  const result = new Map<number, Wire>();
  for (const [index, raw] of array(bindings.dynamic, "bindings.dynamic").entries()) {
    const binding = record(raw, `dynamic[${index}]`);
    const id = positiveId(binding.id, `dynamic[${index}].id`);
    if (id !== index + 1) fail("dynamic_layout", "dynamic ids are not canonical");
    result.set(id, binding);
  }
  return result;
}

function preflightEffectDomains(
  plan: Wire,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
): void {
  for (const rawPass of array(plan.passes, "plan.passes")) {
    const kind = record(record(rawPass, "plan pass").kind, "plan pass kind");
    if (kind.kind !== "dispatchKernel") continue;
    const invocation = record(kind.invocation, "kernel invocation");
    if (invocation.kind !== "filter" && invocation.kind !== "adjustmentEffect") continue;
    const effect = record(invocation.effect, "effect");
    const kernel = record(effect.kernel, "effect kernel");
    const region = effectRegion(kernel);
    if (region) unitRegion(region);
    const space = record(effect.space, "effect space");
    if (space.kind === "root") continue;
    if (space.kind !== "layer") fail("effect_space", `effect space '${String(space.kind)}' is outside the closed set`);
    const transform = dynamicTransform(dynamic, space.transform);
    const bounds = dynamicBounds(dynamic, space.bounds);
    if (kernel.kind === "mosaic" && !project3(transform, [0, 0])) {
      fail("effect_space", `effect '${String(effect.semanticPath)}' has an invalid layer origin`);
    }
    if (region && deviceRectIntersectsRoot(bounds, extent) && !inverse3(transform)) {
      fail("effect_space", `effect '${String(effect.semanticPath)}' has a non-invertible layer transform`);
    }
  }
}

function deviceRectIntersectsRoot(
  bounds: Wire,
  extent: { width: number; height: number },
): boolean {
  const x = finiteNumber(bounds.x, "effect bounds.x");
  const y = finiteNumber(bounds.y, "effect bounds.y");
  const width = positiveIdOrZero(bounds.width, "effect bounds.width");
  const height = positiveIdOrZero(bounds.height, "effect bounds.height");
  return width > 0 && height > 0 && x < extent.width && y < extent.height && x + width > 0 && y + height > 0;
}

function dynamicScalar(dynamic: Map<number, Wire>, id: unknown): number {
  const value = record(required(dynamic, positiveId(id, "dynamic scalar id"), "dynamic scalar").value, "dynamic scalar value");
  if (value.kind !== "scalar") fail("dynamic_type", "dynamic binding is not scalar");
  return finiteNumber(value.value, "dynamic scalar");
}

function dynamicTransform(dynamic: Map<number, Wire>, id: unknown): number[] {
  const value = record(required(dynamic, positiveId(id, "dynamic transform id"), "dynamic transform").value, "dynamic transform value");
  if (value.kind !== "deviceTransform") fail("dynamic_type", "dynamic binding is not DeviceTransform");
  return numberArray(record(value.value, "DeviceTransform").matrix, 9, "DeviceTransform.matrix");
}

function dynamicBounds(dynamic: Map<number, Wire>, id: unknown): Wire {
  const value = record(required(dynamic, positiveId(id, "dynamic bounds id"), "dynamic bounds").value, "dynamic bounds value");
  if (value.kind !== "bounds") fail("dynamic_type", "dynamic binding is not Bounds");
  return record(value.value, "DeviceRect");
}

function renderExtent(plan: Wire): { width: number; height: number } {
  const spec = record(plan.renderSpec, "plan.renderSpec");
  return { width: positiveId(spec.width, "render width"), height: positiveId(spec.height, "render height") };
}

function preflightTarget(surface: Surface, extent: { width: number; height: number }): void {
  const canvas = surface.getCanvas();
  const bounds = canvas.getDeviceClipBounds();
  if (bounds[2] - bounds[0] !== extent.width || bounds[3] - bounds[1] !== extent.height) fail("target_extent", "target extent does not match RenderSpec");
}

function externalObject(resources: Map<number, CanvasKitExternalObject>, id: unknown): CanvasKitExternalObject {
  const object = required(resources, positiveId(id, "external resource id"), "external resource");
  if (!object.image || (object.kind !== "visual" && object.kind !== "scene3d")) fail("external_type", "external resource is not an image");
  return object;
}

function resourceValue(values: Map<number, ImageValue>, id: unknown): ImageValue {
  return required(values, positiveId(id, "plan resource id"), "plan resource");
}
function image(values: Map<number, ImageValue>, id: unknown): Image | null { return resourceValue(values, id).image; }
function programValue(values: Map<number, ImageValue>, id: unknown): ImageValue {
  return required(values, positiveId(id, "program resource id"), "program resource");
}
function borrowed(image: Image | null, roi: DeviceRoi): ImageValue { return { image, owned: false, roi }; }
function borrowedValue(value: ImageValue): ImageValue { return { image: value.image, owned: false, roi: value.roi }; }
function transparentValue(): ImageValue { return { image: null, owned: false, roi: { x: 0, y: 0, width: 0, height: 0 } }; }
function liveImages(values: Map<number, ImageValue>): number { return new Set([...values.values()].map((value) => value.image).filter(Boolean)).size; }

function drawImageValue(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  value: ImageValue,
  alpha: number,
  mode: string,
): void {
  if (!value.image || value.roi.width === 0 || value.roi.height === 0) return;
  const paint = new CanvasKit.Paint();
  try {
    paint.setAlphaf(alpha);
    drawImageValueWithPaint(CanvasKit, canvas, value, paint, mode);
  } finally { paint.delete(); }
}

function drawImageValueWithPaint(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  value: ImageValue,
  paint: Paint,
  mode: string,
): void {
  if (!value.image || value.roi.width === 0 || value.roi.height === 0) return;
  paint.setBlendMode(blendMode(CanvasKit, mode));
  const source = CanvasKit.XYWHRect(0, 0, value.roi.width, value.roi.height);
  const destination = CanvasKit.XYWHRect(value.roi.x, value.roi.y, value.roi.width, value.roi.height);
  canvas.drawImageRectOptions(
    value.image,
    source,
    destination,
    CanvasKit.FilterMode.Nearest,
    CanvasKit.MipmapMode.None,
    paint,
  );
}

function materializeRootValue(
  CanvasKit: CanvasKit,
  arena: SurfaceArena,
  value: ImageValue,
  extent: { width: number; height: number },
): ImageValue {
  const root: DeviceRoi = { x: 0, y: 0, ...extent };
  if (value.image === null) return borrowed(null, root);
  if (sameRoi(value.roi, root)) return borrowedValue(value);
  const surface = arena.transient();
  try {
    const canvas = surface.getCanvas();
    canvas.clear(CanvasKit.TRANSPARENT);
    drawImageValue(CanvasKit, canvas, value, 1, "src");
    surface.flush();
    return { image: surface.makeImageSnapshot(), owned: true, roi: root };
  } finally {
    arena.releaseTransient(surface);
  }
}

function materializeRootResources(
  CanvasKit: CanvasKit,
  arena: SurfaceArena,
  values: Map<number, ImageValue>,
  extent: { width: number; height: number },
): Map<number, ImageValue> {
  return new Map([...values].map(([id, value]) => [
    id,
    materializeRootValue(CanvasKit, arena, value, extent),
  ]));
}

function disposeMaterializedResources(
  materialized: Map<number, ImageValue>,
  originals: Map<number, ImageValue>,
): void {
  for (const [id, value] of materialized) {
    if (value.owned && value.image !== originals.get(id)?.image) value.image?.delete();
  }
  materialized.clear();
}

function drawImage(CanvasKit: CanvasKit, canvas: Canvas, image: Image | null, alpha: number, mode: string): void {
  if (!image) return;
  const paint = new CanvasKit.Paint();
  try {
    paint.setAlphaf(alpha);
    paint.setBlendMode(blendMode(CanvasKit, mode));
    canvas.drawImage(image, 0, 0, paint);
  } finally { paint.delete(); }
}

function drawImageWithPaint(CanvasKit: CanvasKit, canvas: Canvas, image: Image | null, paint: Paint, mode: string): void {
  if (!image) return;
  paint.setBlendMode(blendMode(CanvasKit, mode));
  canvas.drawImage(image, 0, 0, paint);
}

function blendMode(CanvasKit: CanvasKit, mode: string): BlendMode {
  const names: Record<string, string> = {
    src: "Src", srcOver: "SrcOver", dstIn: "DstIn", dstOut: "DstOut",
    normal: "SrcOver", multiply: "Multiply", screen: "Screen", overlay: "Overlay",
    darken: "Darken", lighten: "Lighten", colorDodge: "ColorDodge", colorBurn: "ColorBurn",
    hardLight: "HardLight", softLight: "SoftLight", difference: "Difference", exclusion: "Exclusion",
    hue: "Hue", saturation: "Saturation", color: "Color", luminosity: "Luminosity",
    linearBurn: "SrcOver", dstInLuminance: "DstIn",
  };
  const name = names[mode];
  if (!name) fail("blend_mode", `blend mode '${mode}' is not closed`);
  return (CanvasKit.BlendMode as unknown as Record<string, BlendMode>)[name]!;
}

function imageShader(CanvasKit: CanvasKit, image: Image, sampling: string): Shader {
  const shader = image.makeShaderOptions(CanvasKit.TileMode.Clamp, CanvasKit.TileMode.Clamp, sampling === "nearestClamp" ? CanvasKit.FilterMode.Nearest : CanvasKit.FilterMode.Linear, CanvasKit.MipmapMode.None);
  if (!shader) fail("image_shader", "image cannot become a shader child");
  return shader;
}

function imageValueShader(CanvasKit: CanvasKit, value: ImageValue, sampling: string): Shader {
  if (!value.image) return imageValueOrTransparentShader(CanvasKit, value);
  const shader = value.image.makeShaderOptions(
    CanvasKit.TileMode.Clamp,
    CanvasKit.TileMode.Clamp,
    sampling === "nearestClamp" ? CanvasKit.FilterMode.Nearest : CanvasKit.FilterMode.Linear,
    CanvasKit.MipmapMode.None,
    [1, 0, value.roi.x, 0, 1, value.roi.y, 0, 0, 1],
  );
  if (!shader) fail("image_shader", "ROI image cannot become a shader child");
  return shader;
}

function decodeBase64Bytes(value: unknown, label: string): Uint8Array {
  if (typeof value !== "string") fail("program_contract", `${label} must be a base64 string`);
  let decoded: string;
  try {
    decoded = atob(value);
  } catch {
    fail("program_contract", `${label} is not valid base64`);
  }
  const bytes = new Uint8Array(decoded.length);
  for (let index = 0; index < decoded.length; index += 1) bytes[index] = decoded.charCodeAt(index);
  return bytes;
}

function normalizedExternalShader(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  object: CanvasKitExternalObject,
  sampling = "linearClamp",
): Shader {
  if (!object.image) fail("external_type", "normalized external shader needs an image");
  let primaries = "rec709";
  let transfer = "srgb";
  let referenceWhite = 100;
  let peak = 100;
  if (object.kind === "visual") {
    const key = record(object.key, "visual resource key");
    const resourceInterpretation = record(key.interpretation, "visual resource interpretation");
    if (resourceInterpretation.kind !== "visual") fail("external_type", "visual object key has a non-visual interpretation");
    const interpretation = record(resourceInterpretation.interpretation, "visual interpretation");
    const color = record(interpretation.color, "visual color description");
    const luminance = record(interpretation.luminance, "visual signal luminance");
    primaries = String(color.primaries);
    transfer = String(color.transfer);
    referenceWhite = positiveId(luminance.referenceWhite, "visual reference white");
    peak = positiveId(luminance.peak, "visual peak luminance");
  } else if (object.kind !== "scene3d") {
    fail("external_type", "only visual and Scene3D images enter the working color domain");
  }
  const child = imageShader(CanvasKit, object.image, sampling);
  try {
    return builtins.shader(
      "normalizeInput",
      [transferCode(transfer), primariesCode(primaries), referenceWhite, peak],
      [child],
    );
  } finally { child.delete(); }
}

function transferCode(value: string): number {
  if (value === "linear") return 0;
  if (value === "srgb") return 1;
  if (value === "rec709") return 2;
  if (value === "pq") return 3;
  if (value === "hlg") return 4;
  fail("color_transfer", `transfer '${value}' is outside the closed set`);
}

function primariesCode(value: string): number {
  if (value === "rec709") return 0;
  if (value === "displayP3") return 1;
  if (value === "rec2020") return 2;
  fail("color_primaries", `primaries '${value}' are outside the closed set`);
}

function linearColor(CanvasKit: CanvasKit, color: LinearColorWire): Float32Array {
  const alpha = finiteNumber(color.alpha, "color alpha");
  if (alpha <= 0) return CanvasKit.Color4f(0, 0, 0, 0);
  return CanvasKit.Color4f(color.red / alpha, color.green / alpha, color.blue / alpha, alpha);
}

function color4(value: unknown): Float32Array {
  const channels = numberArray(value, 4, "color");
  const alpha = channels[3]!;
  if (alpha <= 0) return Float32Array.from([0, 0, 0, 0]);
  return Float32Array.from([channels[0]! / alpha, channels[1]! / alpha, channels[2]! / alpha, alpha]);
}

function skRect(CanvasKit: CanvasKit, value: RectWire) { return CanvasKit.XYWHRect(value.x, value.y, value.width, value.height); }
function deviceRect(CanvasKit: CanvasKit, value: Wire) { return CanvasKit.XYWHRect(finiteNumber(value.x, "rect.x"), finiteNumber(value.y, "rect.y"), positiveIdOrZero(value.width, "rect.width"), positiveIdOrZero(value.height, "rect.height")); }
function rect(value: unknown, label: string): RectWire { const wire = record(value, label); return { x: finiteNumber(wire.x, `${label}.x`), y: finiteNumber(wire.y, `${label}.y`), width: finiteNumber(wire.width, `${label}.width`), height: finiteNumber(wire.height, `${label}.height`) }; }
function pair(value: unknown, label: string): [number, number] { const values = numberArray(value, 2, label); return [values[0]!, values[1]!]; }
function numberArray(value: unknown, length: number, label: string): number[] { const values = array(value, label).map((item) => finiteNumber(item, label)); if (values.length !== length) fail("wire_shape", `${label} must contain ${length} numbers`); return values; }
function finiteNumber(value: unknown, label: string): number { if (typeof value !== "number" || !Number.isFinite(value)) fail("wire_number", `${label} must be finite`); return value; }
function positiveId(value: unknown, label: string): number { const result = positiveIdOrZero(value, label); if (result === 0) fail("wire_id", `${label} must be positive`); return result; }
function positiveIdOrZero(value: unknown, label: string): number { const number = typeof value === "bigint" ? Number(value) : value; if (!Number.isSafeInteger(number) || Number(number) < 0) fail("wire_id", `${label} must be an exact non-negative integer`); return Number(number); }
function exactPositiveInteger(value: unknown, label: string): bigint { const number = typeof value === "bigint" ? value : BigInt(positiveId(value, label)); if (number <= 0n) fail("wire_id", `${label} must be positive`); return number; }
function byte(value: unknown, label: string): number { const result = positiveIdOrZero(value, label); if (result > 255) fail("wire_byte", `${label} exceeds 255`); return result; }
function array(value: unknown, label: string): unknown[] { if (!Array.isArray(value)) fail("wire_shape", `${label} must be an array`); return value; }
function record(value: unknown, label: string): Wire { if (typeof value !== "object" || value === null || Array.isArray(value)) fail("wire_shape", `${label} must be an object`); return value as Wire; }
function passOutput(kind: Wire): number { if (kind.kind === "dispatchKernel") return positiveId(record(kind.invocation, "kernel invocation").output, "kernel output"); return positiveId(kind.output, "pass output"); }
function required<K, V>(map: ReadonlyMap<K, V>, key: K, label: string): V { const value = map.get(key); if (value === undefined) fail("missing_value", `${label} '${String(key)}' is absent`); return value; }
function requiredIndex<T>(values: T[], id: unknown, label: string): T { const index = positiveIdOrZero(id, `${label} id`); const value = values[index]; if (value === undefined) fail("missing_value", `${label} ${index} is absent`); return value; }
function fontKey(hash: string, index: number): string { return `${hash}:${index}`; }
/** @internal Shared by executor admission and its source-level contract tests. */
export function admitProgramTextures(
  requirementsValue: unknown,
  bindingsValue: unknown,
  objects: ReadonlyMap<number, CanvasKitExternalObject>,
): Map<string, CanvasKitExternalObject> {
  const requiredTextures = array(requirementsValue, "program texture requirements");
  const textureBindings = array(bindingsValue, "program textures");
  if (textureBindings.length !== requiredTextures.length) {
    fail("texture_contract", "program texture bindings do not match requirements");
  }
  const textures = new Map<string, CanvasKitExternalObject>();
  for (let index = 0; index < textureBindings.length; index += 1) {
    const binding = record(textureBindings[index], "texture binding");
    const requirement = record(requiredTextures[index], "texture requirement");
    if (String(binding.key) !== String(requirement.key)) {
      fail("texture_contract", "program texture binding order/key is not canonical");
    }
    const object = required(objects, positiveId(binding.slot, "texture slot"), "texture slot");
    if (object.kind !== "visual" || !object.image) {
      fail("texture_contract", `texture '${String(binding.key)}' is not visual`);
    }
    const identity = textureIdentity(requirement);
    if (textures.has(identity)) {
      fail("texture_contract", `duplicate program texture '${String(binding.key)}'`);
    }
    textures.set(identity, object);
  }
  return textures;
}

/** @internal Resolves the same full identity used by image and shader execution. */
export function resolveProgramTexture(
  textures: ReadonlyMap<string, CanvasKitExternalObject>,
  textureValue: unknown,
  label = "program texture",
): CanvasKitExternalObject {
  return required(textures, textureIdentity(record(textureValue, `${label} requirement`)), label);
}

function textureIdentity(texture: Wire): string {
  const kind = String(texture.kind);
  const sample = texture.sampleTimeMicros;
  if (sample !== null && sample !== undefined && (typeof sample !== "number" || !Number.isSafeInteger(sample) || sample < 0)) {
    fail("texture_contract", "texture sampleTimeMicros must be a non-negative safe integer or null");
  }
  if (kind === "video") {
    if (sample === null || sample === undefined) {
      fail("texture_contract", "video textures must carry sampleTimeMicros");
    }
  } else if (kind === "image" || kind === "generated") {
    if (sample !== null && sample !== undefined) {
      fail("texture_contract", "only video textures may carry sampleTimeMicros");
    }
  } else {
    fail("texture_contract", `texture kind '${kind}' is outside the closed set`);
  }
  return `${String(texture.key)}\u0000${sample === null || sample === undefined ? "static" : String(sample)}`;
}
function deepEqual(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (typeof left !== typeof right) return false;
  if (Array.isArray(left) || Array.isArray(right)) {
    return Array.isArray(left)
      && Array.isArray(right)
      && left.length === right.length
      && left.every((value, index) => deepEqual(value, right[index]));
  }
  if (typeof left !== "object" || left === null || right === null) return false;
  const leftRecord = left as Record<string, unknown>;
  const rightRecord = right as Record<string, unknown>;
  const leftKeys = Object.keys(leftRecord);
  const rightKeys = Object.keys(rightRecord);
  return leftKeys.length === rightKeys.length
    && leftKeys.every((key) => Object.hasOwn(rightRecord, key)
      && deepEqual(leftRecord[key], rightRecord[key]));
}
function bigintJson(_key: string, value: unknown): unknown { return typeof value === "bigint" ? value.toString() : value; }

function mul3(left: number[], right: number[]): number[] {
  const output = Array<number>(9).fill(0);
  for (let row = 0; row < 3; row += 1) for (let column = 0; column < 3; column += 1) for (let index = 0; index < 3; index += 1) output[row * 3 + column]! += left[row * 3 + index]! * right[index * 3 + column]!;
  return output;
}

function inverse3(value: number[]): number[] | null {
  const [a, b, c, d, e, f, g, h, i] = value as [number, number, number, number, number, number, number, number, number];
  const cofactors = [e * i - f * h, c * h - b * i, b * f - c * e, f * g - d * i, a * i - c * g, c * d - a * f, d * h - e * g, b * g - a * h, a * e - b * d];
  const determinant = a * cofactors[0]! + b * cofactors[3]! + c * cofactors[6]!;
  return Number.isFinite(determinant) && Math.abs(determinant) > 1e-15 ? cofactors.map((item) => item / determinant) : null;
}

function project3(matrix: number[], point: [number, number]): [number, number] | null {
  const w = matrix[6]! * point[0] + matrix[7]! * point[1] + matrix[8]!;
  if (!Number.isFinite(w) || Math.abs(w) <= 1e-12) return null;
  const result: [number, number] = [
    (matrix[0]! * point[0] + matrix[1]! * point[1] + matrix[2]!) / w,
    (matrix[3]! * point[0] + matrix[4]! * point[1] + matrix[5]!) / w,
  ];
  return result.every(Number.isFinite) ? result : null;
}

function maxBigInt(left: bigint, right: bigint): bigint {
  return left > right ? left : right;
}

function fail(code: string, message: string): never { throw new CanvasKitCompositorError(code, message); }
