import { artifactFontUrls } from "./font-demand.ts";
import {
  createTimelineCompilerRuntime,
  MotionCompileError,
  type MotionCompileOptions,
  type MotionCompilerDiagnostic,
  type PreparedPreviewPackage,
  type PreviewResourceInput,
  type Timeline,
  type WebCompilerRuntime,
} from "valle-engine";

import { digestBytes, motionCompileOptionIdentity, prepareAudioAnalysisInputs, type AudioAnalysisInput } from "./audio-analysis-input.ts";

export interface StudioCompileInstance {
  clipPath: string;
  entry: string;
  modules: Record<string, string>;
  options?: MotionCompileOptions;
  audioUrls?: readonly AudioAnalysisInput[];
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
      timings: { runtimeMs: number; fontsMs: number; compileMs: number; prepareMs: number; compilations: number; cacheHit: boolean };
      warnings: MotionCompilerDiagnostic[];
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
    if (request.authorTimeline) {
      const prepared = new Map(compiler.motionPreparationInputs(request.authorTimeline).map(input => [input.clipPath, input]));
      request = { ...request, instances: request.instances.map(instance => {
        const input = prepared.get(instance.clipPath);
        if (!input) throw new Error(`Motion preparation input is missing for ${instance.clipPath}`);
        return { ...instance, options: { ...instance.options, data: input.data === null ? undefined : { source: `timeline:${instance.clipPath}`, value: input.data } } };
      }) };
    }
    // Fetch canonical PCM in this long-lived Worker. Source edits only send URLs
    // and digests from the UI; they do not clone the song through postMessage.
    request = { ...request, instances: await Promise.all(request.instances.map(async instance => {
      if (!instance.audioUrls?.length || !Object.values(instance.modules).some(source => source.includes("audioAnalysis"))) return instance;
      const audioSources = await prepareAudioAnalysisInputs(instance.audioUrls);
      return { ...instance, options: { ...instance.options, audioSources } };
    })) };
    const runtimeMs = performance.now() - started;
    const byteDigests = new WeakMap<Uint8Array, Promise<string>>();
    const digest = (bytes: Uint8Array): Promise<string> => {
      let pending = byteDigests.get(bytes);
      if (!pending) { pending = digestBytes(bytes); byteDigests.set(bytes, pending); }
      return pending;
    };
    const optionIdentities = await Promise.all(request.instances.map(instance => motionCompileOptionIdentity(instance.options, digest)));
    let compilations = 0;
    const currentCompilations = new Map<string, ReturnType<WebCompilerRuntime["compileMotionModules"]>>();
    const preliminary = new Map<number, ReturnType<WebCompilerRuntime["compileMotionModules"]>>();
    const hydrated = await Promise.all(request.instances.map(async (instance, index) => {
      if (instance.fonts || !instance.fontUrls?.length) return instanceFonts(instance);
      const key = JSON.stringify([instance.entry, instance.modules, optionIdentities[index], "without-measurement-fonts"]);
      let compiled = currentCompilations.get(key) ?? compiledInstances.get(key);
      if (!compiled) {
        try {
          compilations += 1;
          compiled = compiler.compileMotionModules(instance.entry, instance.modules, { ...instance.options, fonts: [] });
          currentCompilations.set(key, compiled);
          compiledInstances.set(key, compiled);
          if (compiledInstances.size > 32) compiledInstances.delete(compiledInstances.keys().next().value!);
        } catch (error) {
          // The first compile has no measurement environment. Retry only when author code needs
          // a measurement/outline helper; rendering-only text can select fonts from its artifact.
          if (!/measureText|textOutline|measure font|font bytes/iu.test(String(error))) throw error;
          return instanceFonts(instance);
        }
      }
      preliminary.set(index, compiled);
      return instanceFonts({ ...instance, fontUrls: artifactFontUrls(compiled.artifact, instance.fontUrls) });
    }));
    const fontsMs = performance.now() - started - runtimeMs;
    const cacheKey = JSON.stringify([request.authorTimeline, request.standalone,
      request.instances.map((instance, index) => [instance.clipPath, instance.entry, instance.modules,
        optionIdentities[index], hydrated[index]!.digests]), request.resourceInputs]);
    if (cachedInputs === cacheKey && cachedResult?.status === "ok") {
      return { ...cachedResult, id: request.id,
        timings: { runtimeMs, fontsMs, compileMs: 0, prepareMs: 0, compilations, cacheHit: true } };
    }
    const compileStarted = performance.now();
    const instances = request.instances.map((instance, index) => {
      const fonts = hydrated[index]!.fonts;
      const instanceKey = JSON.stringify([instance.entry, instance.modules, optionIdentities[index],
        hydrated[index]!.digests]);
      let compiled = preliminary.get(index) ?? currentCompilations.get(instanceKey) ?? compiledInstances.get(instanceKey);
      if (!compiled) {
        compilations += 1;
        compiled = compiler.compileMotionModules(instance.entry, instance.modules, {
          ...instance.options,
          fonts: fonts.map((font) => font.bytes),
        });
        currentCompilations.set(instanceKey, compiled);
        compiledInstances.set(instanceKey, compiled);
        if (compiledInstances.size > 32) compiledInstances.delete(compiledInstances.keys().next().value!);
      }
      const needed = artifactFontUrls(compiled.artifact, [
        { url: "provided", role: "font" as const }, { url: "provided", role: "formula-font" as const },
      ]);
      const roles = new Set(needed.map(font => font.role));
      return { clipPath: instance.clipPath, ...compiled,
        fonts: fonts.filter((font) => roles.has(font.role))
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
      timings: { runtimeMs, fontsMs, compileMs, prepareMs, compilations, cacheHit: false },
      warnings: [...new Map(instances.flatMap((instance) => instance.warnings)
        .map((warning) => [JSON.stringify([warning.sourcePath, warning.span, warning.code, warning.message]), warning])).values()],
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
