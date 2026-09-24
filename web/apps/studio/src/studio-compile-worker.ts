import {
  createTimelineCompilerRuntime,
  MotionCompileError,
  type MotionCompileOptions,
  type PreparedPreviewPackage,
  type PreviewResourceInput,
  type Timeline,
  type WebCompilerRuntime,
} from "valle-engine";

export interface StudioCompileInstance {
  clipPath: string;
  entry: string;
  modules: Record<string, string>;
  options?: MotionCompileOptions;
  fonts?: readonly { bytes: Uint8Array; role: "font" | "formula-font" }[];
  fontUrls?: readonly { url: string; role: "font" | "formula-font" }[];
}

export interface StudioCompileRequest {
  id: number;
  runtimeAssets: unknown;
  runtimeBaseUrl: string;
  authorTimeline?: Timeline;
  standalone?: {
    input: string;
    fpsOverride?: string | null;
    props?: Record<string, unknown>;
    data?: Record<string, unknown>;
    assets?: Array<{ name: string; path: string; alias: string }>;
  };
  instances: StudioCompileInstance[];
  resourceInputs?: PreviewResourceInput[];
}

export type StudioCompileResult =
  | { id: number; status: "ok"; authorTimeline: Timeline; package: PreparedPreviewPackage;
      timings: { runtimeMs: number; fontsMs: number; compileMs: number; prepareMs: number; cacheHit: boolean };
      instances: Array<{ clipPath: string; artifact: Record<string, unknown>; artifactDigest: string;
        sourceMap: Record<string, unknown> }> }
  | { id: number; status: "error"; message: string; diagnostics?: MotionCompileError["diagnostics"] };

let runtime: Promise<WebCompilerRuntime> | null = null;
let runtimeIdentity = "";
let cachedInputs = "";
let cachedResult: StudioCompileResult | null = null;
const compiledInstances = new Map<string, ReturnType<WebCompilerRuntime["compileMotionModules"]>>();
const immutableFonts = new Map<string, Promise<Uint8Array>>();

async function fontAt(url: string): Promise<Uint8Array> {
  const immutable = url.startsWith("/runtime/fonts/");
  let pending = immutable ? immutableFonts.get(url) : undefined;
  if (!pending) {
    pending = fetch(url).then(async (response) => {
      if (!response.ok) throw new Error(`Font ${url} returned ${response.status}`);
      return new Uint8Array(await response.arrayBuffer());
    });
    if (immutable) immutableFonts.set(url, pending);
    void pending.catch(() => { if (immutable && immutableFonts.get(url) === pending) immutableFonts.delete(url); });
  }
  return pending;
}

async function digestBytes(bytes: Uint8Array): Promise<string> {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return [...new Uint8Array(await crypto.subtle.digest("SHA-256", copy.buffer))]
    .map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function instanceFonts(instance: StudioCompileInstance): Promise<{
  fonts: Array<{ bytes: Uint8Array; role: "font" | "formula-font" }>;
  digests: string[];
}> {
  const supplied = instance.fonts ?? await Promise.all((instance.fontUrls ?? []).map(async ({ url, role }) => ({
    bytes: await fontAt(url), role,
  })));
  const fingerprints = await Promise.all(supplied.map(async (font) => digestBytes(font.bytes)));
  const seen = new Set<string>();
  const fonts: Array<{ bytes: Uint8Array; role: "font" | "formula-font" }> = [];
  const digests: string[] = [];
  supplied.forEach((font, index) => {
    const digest = fingerprints[index]!;
    if (seen.has(digest)) return;
    seen.add(digest);
    fonts.push(font);
    digests.push(digest);
  });
  return { fonts, digests };
}

function base64(bytes: Uint8Array): string {
  let binary = "";
  for (let at = 0; at < bytes.length; at += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
  }
  return btoa(binary);
}

function standaloneTimeline(
  input: NonNullable<StudioCompileRequest["standalone"]>,
  artifact: Record<string, unknown>,
): Timeline {
  const composition = artifact.composition as {
    width?: unknown; height?: unknown; fps?: unknown; duration?: unknown;
  } | undefined;
  if (!composition || !Number.isInteger(composition.width) || !Number.isInteger(composition.height)
    || typeof composition.duration !== "string") {
    throw new Error("Motion composition needs a valid canvas and duration");
  }
  const fps = input.fpsOverride ?? composition.fps;
  if (typeof fps !== "string" || !fps) {
    throw new Error("Motion composition needs fps when --fps was not supplied");
  }
  const durationParts = composition.duration.split("/").map(Number);
  const duration = durationParts.length === 1 ? durationParts[0]
    : durationParts.length === 2 ? durationParts[0]! / durationParts[1]! : NaN;
  if (!Number.isFinite(duration) || duration <= 0) {
    throw new Error("Motion composition has an invalid duration");
  }
  const resources: Record<string, string> = { motion: input.input };
  const assetBindings: Record<string, string> = {};
  for (const asset of input.assets ?? []) {
    const alias = asset.alias;
    resources[alias] = asset.path;
    assetBindings[asset.name] = alias;
  }
  return {
    canvas: { width: composition.width as number, height: composition.height as number, fps },
    resources,
    tracks: { visual: [{ clips: [{
      kind: "motion", component: "motion", start: 0, duration,
      ...(input.props && Object.keys(input.props).length ? { props: input.props } : {}),
      ...(input.data && Object.keys(input.data).length ? { data: input.data } : {}),
      ...(Object.keys(assetBindings).length ? { resources: assetBindings } : {}),
    }] }] },
  } as Timeline;
}

export async function compileStudioPreview(request: StudioCompileRequest): Promise<StudioCompileResult> {
  try {
    const started = performance.now();
    const identity = JSON.stringify([request.runtimeAssets, request.runtimeBaseUrl]);
    if (identity !== runtimeIdentity) {
      runtimeIdentity = identity;
      runtime = createTimelineCompilerRuntime({
        runtimeAssets: request.runtimeAssets,
        runtimeBaseUrl: request.runtimeBaseUrl,
      });
      cachedResult = null;
      compiledInstances.clear();
    }
    const compiler = await runtime!;
    const runtimeMs = performance.now() - started;
    const hydrated = await Promise.all(request.instances.map(instanceFonts));
    const fontsMs = performance.now() - started - runtimeMs;
    const cacheKey = JSON.stringify([request.authorTimeline, request.standalone,
      request.instances.map((instance, index) => [instance.clipPath, instance.entry, instance.modules,
        instance.options, hydrated[index]!.digests]), request.resourceInputs]);
    if (cachedInputs === cacheKey && cachedResult?.status === "ok") {
      return { ...cachedResult, id: request.id,
        timings: { runtimeMs, fontsMs, compileMs: 0, prepareMs: 0, cacheHit: true } };
    }
    const compileStarted = performance.now();
    const instances = request.instances.map((instance, index) => {
      const fonts = hydrated[index]!.fonts;
      const instanceKey = JSON.stringify([instance.entry, instance.modules, instance.options,
        hydrated[index]!.digests]);
      let compiled = compiledInstances.get(instanceKey);
      if (!compiled) {
        compiled = compiler.compileMotionModules(instance.entry, instance.modules, {
          ...instance.options,
          fonts: fonts.map((font) => font.bytes),
        });
        compiledInstances.set(instanceKey, compiled);
        if (compiledInstances.size > 32) compiledInstances.delete(compiledInstances.keys().next().value!);
      }
      const kinds = new Set((compiled.artifact.nodes as Array<{ kind?: { kind?: string } }> | undefined)
        ?.map((node) => node.kind?.kind));
      return { clipPath: instance.clipPath, ...compiled,
        fonts: fonts.filter((font) => font.role === "formula-font"
          ? kinds.has("mathFormula") : kinds.has("text"))
          .map((font) => ({ bytesBase64: base64(font.bytes), role: font.role })) };
    });
    const compileMs = performance.now() - compileStarted;
    const authorTimeline = request.authorTimeline ?? (
      request.standalone && instances.length === 1
        ? standaloneTimeline(request.standalone, instances[0]!.artifact)
        : null
    );
    if (!authorTimeline) throw new Error("Studio compile needs an author Timeline");
    const prepareStarted = performance.now();
    const prepared = compiler.preparePreviewPackage({
      authorTimeline,
      motionInstances: instances.map((instance) => ({
        clipPath: instance.clipPath,
        artifact: instance.artifact,
        artifactDigest: instance.artifactDigest,
        fonts: instance.fonts,
      })),
      resourceInputs: request.resourceInputs ?? [],
    });
    const prepareMs = performance.now() - prepareStarted;
    const result: StudioCompileResult = {
      id: request.id, status: "ok", authorTimeline, package: prepared,
      timings: { runtimeMs, fontsMs, compileMs, prepareMs, cacheHit: false },
      instances: instances.map((instance) => ({
        clipPath: instance.clipPath, artifact: instance.artifact,
        artifactDigest: instance.artifactDigest, sourceMap: instance.sourceMap,
      })),
    };
    cachedInputs = cacheKey;
    cachedResult = result;
    return result;
  } catch (error) {
    return { id: request.id, status: "error",
      message: error instanceof Error ? error.message : String(error),
      ...(error instanceof MotionCompileError ? { diagnostics: error.diagnostics } : {}),
    };
  }
}

if (typeof self !== "undefined" && "postMessage" in self && "onmessage" in self) {
  self.onmessage = (event: MessageEvent<StudioCompileRequest>) => {
    void compileStudioPreview(event.data).then((result) => self.postMessage(result));
  };
}
