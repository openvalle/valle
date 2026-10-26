import type { PreviewResourceInput, Timeline } from "valle-engine";
import type { ResourceManifest, TimelineDocument } from "valle-engine/internal";
import { type StudioHost, type TimelinePreviewResult } from "./host.ts";
import type { StudioSourceEditor } from "./source-editor.ts";
import { StudioCompileClient, timelineCompilePayload } from "./studio-compile.ts";

/** Only bindings and media sources can change the frozen facts supplied by Host. */
export function mediaInputKey(timeline: Timeline): string | null {
  const uses = new Map<string, Set<string>>();
  const use = (alias: string, kind: string) => {
    let kinds = uses.get(alias);
    if (!kinds) uses.set(alias, (kinds = new Set()));
    kinds.add(kind);
  };
  const bindings = (resources?: Record<string, string> | null) => {
    for (const alias of Object.values(resources ?? {})) use(alias, "motion");
  };
  for (const track of timeline.tracks.visual ?? []) {
    for (const clip of track.clips) {
      if (clip.kind === "motion") {
        bindings(clip.resources);
      } else if (clip.kind === "image" || clip.kind === "video") {
        use(clip.src, clip.kind);
        if (clip.kind === "video" && clip.gain !== 0) use(clip.src, "audio");
      }
      for (const effect of clip.effects ?? []) {
        bindings(effect.resources);
      }
    }
    for (const transition of track.transitions ?? []) {
      if (transition.kind === "motion") {
        bindings(transition.resources);
      }
    }
  }
  for (const track of timeline.tracks.caption ?? []) {
    use(track.style.font, "font");
    if (track.presenter) {
      bindings(track.presenter.resources);
    }
  }
  for (const clip of (timeline.tracks.adjustment ?? []).flatMap((track) => track.clips)) {
    if (clip.kind === "motion") {
      bindings(clip.resources);
    }
  }
  for (const track of timeline.tracks.audio ?? []) {
    for (const clip of track.clips) use(clip.src, "audio");
  }
  if (!uses.size) return null;
  return JSON.stringify(
    [...uses]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([alias, kinds]) => [alias, timeline.resources?.[alias] ?? null, [...kinds].sort()]),
  );
}

/** Project and Timeline use the same browser preparation as standalone Motion. */
export class TimelineBrowserSession {
  readonly #host: StudioHost;
  readonly #sources: StudioSourceEditor;
  readonly #compiler: StudioCompileClient;
  #mediaKey: string | null = null;
  #resourceInputs: PreviewResourceInput[] = [];
  #locators: Array<{ id: string; url: string }> = [];
  #mediaDependencies: Record<string, string> = {};
  #resourceBytesSource?: { digest: string; url: string };

  constructor(host: StudioHost, sources: StudioSourceEditor) {
    if (!host.loadMediaFacts) throw new Error("Studio media facts are unavailable");
    this.#host = host;
    this.#sources = sources;
    this.#compiler = new StudioCompileClient(host.boot);
  }

  close(): void {
    this.#compiler.close();
  }

  invalidateMedia(): void {
    this.#mediaKey = null;
  }

  async prepareTimelinePreview(timeline: Timeline): Promise<TimelinePreviewResult> {
    try {
      await this.#prepareMedia(timeline);
      const sources = this.#sources.snapshot();
      const sourceDependencies = this.#sources.confirmedDigests();
      const resourceInputs = this.#resourceInputs;
      const locators = this.#locators;
      const mediaDependencies = this.#mediaDependencies;
      const result = await this.#compiler.compile(
        timelineCompilePayload(
          this.#host.boot,
          timeline,
          sources,
          resourceInputs,
          locators,
          this.#resourceBytesSource,
        ),
      );
      if (result.status === "error") throw new Error(result.message);
      const render = {
        timelineJson: result.package.timelineJson,
        timeline: JSON.parse(result.package.timelineJson) as TimelineDocument,
        fixedPackageManifestJson: result.package.fixedPackageManifestJson,
        resourceManifestJson: result.package.resourceManifestJson,
        resourceManifest: JSON.parse(result.package.resourceManifestJson) as ResourceManifest,
        verifiedBindingBundleJson: result.package.verifiedBindingBundleJson,
        resourceBytes: result.package.resourceBytes,
      };
      return {
        status: "ok",
        timeline,
        timelineJson: JSON.stringify(timeline),
        render,
        assets: locators,
        motion: { structures: [] },
        motionSourceMetadata: result.package.motionSourceMetadata,
        inputDependencies: { ...mediaDependencies, ...sourceDependencies },
        motionInstances: result.instances,
        warnings: result.warnings,
      };
    } catch (error) {
      return {
        status: "error",
        diagnostics: [
          {
            class: "compile",
            code: "studio-preview",
            message: error instanceof Error ? error.message : String(error),
          },
        ],
      };
    }
  }

  async #prepareMedia(timeline: Timeline): Promise<void> {
    const key = mediaInputKey(timeline);
    if (key === null) {
      this.#mediaKey = null;
      this.#resourceInputs = [];
      this.#locators = [];
      this.#mediaDependencies = {};
      this.#resourceBytesSource = undefined;
      return;
    }
    if (key === this.#mediaKey) return;
    const began = performance.now();
    const preview = await this.#host.loadMediaFacts!({ timeline });
    if (preview.status === "error") {
      throw new Error(preview.diagnostics.map((item) => item.message).join("\n"));
    }
    this.#resourceInputs = preview.resources;
    performance.mark("valle-studio-media-facts", {
      detail: { roundTripMs: performance.now() - began, ...preview.timings },
    });
    this.#locators = preview.assets;
    this.#mediaDependencies = preview.inputDependencies;
    this.#resourceBytesSource = preview.resourceBytes;
    this.#mediaKey = key;
  }
}
