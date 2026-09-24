import type { MotionCompileOptions, PreviewResourceInput, Timeline } from "valle-engine";
import { playerRuntimeAssetsFromStudioBoot, type StudioBoot } from "./host.ts";
import type {
  StudioCompileRequest,
  StudioCompileResult,
} from "./studio-compile-worker.ts";
import type { StandaloneAssets } from "./standalone-assets.ts";
import { frozenShaderBytes } from "./shader-input.ts";

export type CompilePayload = Omit<StudioCompileRequest, "id">;

/** The source closure uses project-relative names even though Host identifies files absolutely. */
export function moduleClosure(entryPath: string, sources: Record<string, string>): {
  entry: string; modules: Record<string, string>;
} {
  const slash = entryPath.lastIndexOf("/");
  const entry = entryPath.slice(slash + 1);
  if (!entry) throw new Error("Motion entry path has no file name");
  const candidates = Object.keys(sources).filter((path) => path === entryPath || path.endsWith(`/${entry}`));
  const actualEntry = candidates.includes(entryPath) ? entryPath : candidates.length === 1 ? candidates[0] : null;
  if (!actualEntry) throw new Error(`Motion source is unavailable or ambiguous: ${entryPath}`);
  const modulePaths = Object.keys(sources).filter((path) => /\.(?:tsx?|jsx?)$/.test(path));
  let root = actualEntry.slice(0, actualEntry.lastIndexOf("/") + 1);
  while (modulePaths.some((path) => !path.startsWith(root)) && root !== "/") {
    root = root.slice(0, root.slice(0, -1).lastIndexOf("/") + 1);
  }
  const modules: Record<string, string> = {};
  for (const [path, text] of Object.entries(sources)) {
    if (!path.startsWith(root)) continue;
    const relative = path.slice(root.length);
    if (/\.(?:tsx?|jsx?)$/.test(relative)) modules[relative] = text;
  }
  const relativeEntry = actualEntry.slice(root.length);
  if (!(relativeEntry in modules)) throw new Error(`Motion source is unavailable: ${entryPath}`);
  return { entry: relativeEntry, modules };
}

function decodeBase64(value: string): Uint8Array {
  return Uint8Array.from(atob(value), (character) => character.charCodeAt(0));
}

function sourceTextForPath(path: string | undefined, sources: Record<string, string>): string | undefined {
  if (!path) return undefined;
  if (path in sources) return sources[path];
  const basename = path.split(/[\\/]/).pop();
  const matches = Object.keys(sources).filter((candidate) => candidate.endsWith(`/${basename}`));
  if (matches.length > 1) throw new Error(`Source path is ambiguous: ${path}`);
  return matches.length === 1 ? sources[matches[0]!] : undefined;
}

export function timelineCompilePayload(
  boot: StudioBoot,
  timeline: Timeline,
  sources: Record<string, string>,
  resourceInputs: readonly PreviewResourceInput[],
): CompilePayload {
  const resources = new Map(resourceInputs.map((resource) => [resource.id, resource]));
  const instances: CompilePayload["instances"] = [];
  for (const [trackIndex, track] of (timeline.tracks.visual ?? []).entries()) {
    for (const [clipIndex, clip] of track.clips.entries()) {
      if (clip.kind !== "motion") continue;
      const locator = timeline.resources?.[clip.component];
      if (typeof locator !== "string") throw new Error(`Motion resource ${clip.component} is missing`);
      const { entry, modules } = moduleClosure(locator, sources);
      const options: MotionCompileOptions = {};
      if (clip.data) options.data = { source: `timeline:${trackIndex}:${clipIndex}`, value: clip.data };
      const refs = Object.entries(clip.resources ?? {}).map(([control, alias]) => {
        const resource = resources.get(`resource:${alias}`);
        if (!resource || typeof resource.entry.digest !== "string") {
          throw new Error(`Motion asset ${control} has no frozen resource facts`);
        }
        return { control, contentHash: resource.entry.digest };
      });
      if (refs.length) options.resources = refs;
      const aliases: Record<string, Uint8Array> = {};
      for (const [control, alias] of Object.entries(clip.resources ?? {})) {
        const resource = resources.get(`resource:${alias}`);
        if (resource?.entry.kind === "font") {
          const bytes = resource.facts.bytesBase64;
          if (typeof bytes !== "string") throw new Error(`Motion font asset ${control} has no frozen bytes`);
          aliases[`asset://${control}`] = decodeBase64(bytes);
        }
      }
      if (Object.keys(aliases).length) options.fontAliases = aliases;
      const shaders: NonNullable<MotionCompileOptions["shaders"]>[number][] = [];
      for (const [control, alias] of Object.entries(clip.resources ?? {})) {
        const resource = resources.get(`resource:${alias}`);
        if (resource?.entry.kind !== "shader") continue;
        const manifest = resource.facts.manifestBytesBase64;
        const source = resource.facts.sourceBytesBase64;
        if (typeof manifest !== "string" || typeof source !== "string") {
          throw new Error(`Motion shader asset ${control} has no frozen source package`);
        }
        shaders.push({ assetControl: control,
          frozenBytes: frozenShaderBytes(decodeBase64(manifest), decodeBase64(source)) });
      }
      if (shaders.length) options.shaders = shaders;
      instances.push({ clipPath: `/tracks/visual/${trackIndex}/clips/${clipIndex}`,
        entry, modules, options,
        fontUrls: boot.runtime.fontUrls.map(({ url, role }) => ({ url, role: role as "font" | "formula-font" })) });
    }
  }
  return { runtimeAssets: playerRuntimeAssetsFromStudioBoot(boot), runtimeBaseUrl: location.href,
    authorTimeline: timeline, instances, resourceInputs: [...resourceInputs] };
}

export function standaloneCompilePayload(
  boot: StudioBoot,
  sources: Record<string, string>,
  dataOverride?: Record<string, unknown>,
  propsOverride?: Record<string, unknown>,
  assets?: StandaloneAssets,
): CompilePayload {
  if (boot.session.kind !== "motion-file") throw new Error("Motion file session required");
  const inputs = boot.session.authorInputs;
  if (!inputs) throw new Error("Motion author inputs are unavailable");
  if (inputs.assetSpecs.length && !assets) throw new Error("Browser preview needs frozen asset inputs from Host");
  const { entry, modules } = moduleClosure(boot.session.input, sources);
  const dataText = sourceTextForPath(inputs.dataPath, sources);
  const data = dataOverride ?? (dataText !== undefined ? JSON.parse(dataText) as Record<string, unknown>
    : inputs.data ?? undefined);
  const props = propsOverride ?? inputs.props;
  return {
    runtimeAssets: playerRuntimeAssetsFromStudioBoot(boot),
    runtimeBaseUrl: location.href,
    standalone: { input: boot.session.input, fpsOverride: inputs.fpsOverride, props, data,
      assets: assets?.bindings },
    resourceInputs: assets?.resourceInputs,
    instances: [{
      clipPath: "/tracks/visual/0/clips/0", entry, modules,
      options: { ...(data ? { data: { source: inputs.dataPath ?? "studio:inline", value: data } } : {}),
        ...assets?.options },
      fontUrls: inputs.fontUrls.map(({ url, role }) => ({ url, role: role as "font" | "formula-font" })),
    }],
  };
}

/** A single long-lived Worker serializes compilation and keeps Wasm initialized. */
export class StudioCompileClient {
  readonly #worker: Worker;
  readonly #pending = new Map<number, { resolve: (value: StudioCompileResult) => void;
    reject: (error: Error) => void; started: number }>();
  #nextId = 1;

  constructor(boot: StudioBoot) {
    const path = boot.runtime.assetUrls.studioCompileWorker;
    if (!path) throw new Error("Studio compile Worker is absent from the runtime manifest");
    this.#worker = new Worker(new URL(path, location.href), { type: "module" });
    this.#worker.onmessage = (event: MessageEvent<StudioCompileResult>) => {
      const pending = this.#pending.get(event.data.id);
      if (!pending) return;
      this.#pending.delete(event.data.id);
      if (event.data.status === "ok") {
        performance.mark("valle-studio-compile", { detail: {
          ...event.data.timings, roundTripMs: performance.now() - pending.started,
        } });
      }
      pending.resolve(event.data);
    };
    this.#worker.onerror = (event) => {
      for (const pending of this.#pending.values()) pending.reject(new Error(event.message));
      this.#pending.clear();
    };
  }

  compile(payload: CompilePayload): Promise<StudioCompileResult> {
    const id = this.#nextId++;
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject, started: performance.now() });
      this.#worker.postMessage({ ...payload, id } satisfies StudioCompileRequest);
    });
  }

  close(): void {
    this.#worker.terminate();
    for (const pending of this.#pending.values()) pending.reject(new Error("Studio compiler closed"));
    this.#pending.clear();
  }
}
