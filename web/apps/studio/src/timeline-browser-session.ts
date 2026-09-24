import type { PreviewResourceInput, Timeline } from "valle-engine";
import type { ResourceManifest, TimelineDocument } from "valle-engine/internal";
import {
  type StudioHost,
  type TimelinePreviewResult,
} from "./host.ts";
import type { StudioSourceEditor } from "./source-editor.ts";
import { StudioCompileClient, timelineCompilePayload } from "./studio-compile.ts";

/** Only bindings and media sources can change the frozen facts supplied by Host. */
function mediaInputKey(timeline: Timeline): string | null {
  const motionComponents = new Set<string>();
  const bindings: unknown[] = [];
  const media: unknown[] = [];
  for (const track of timeline.tracks.visual ?? []) {
    for (const clip of track.clips) {
      if (clip.kind === "motion") {
        motionComponents.add(clip.component);
        bindings.push(clip.resources ?? null);
      } else {
        media.push(clip);
      }
    }
  }
  const external = Object.entries(timeline.resources ?? {})
    .filter(([alias]) => !motionComponents.has(alias));
  const otherTracks = { audio: timeline.tracks.audio, caption: timeline.tracks.caption,
    adjustment: timeline.tracks.adjustment };
  if (!external.length && !media.length && Object.values(otherTracks).every((value) => !value?.length)) {
    return null;
  }
  return JSON.stringify([external, bindings, media, otherTracks]);
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

  constructor(host: StudioHost, sources: StudioSourceEditor) {
    if (!host.loadMediaFacts) throw new Error("Studio media facts are unavailable");
    this.#host = host;
    this.#sources = sources;
    this.#compiler = new StudioCompileClient(host.boot);
  }

  close(): void { this.#compiler.close(); }

  invalidateMedia(): void { this.#mediaKey = null; }

  async prepareTimelinePreview(timeline: Timeline): Promise<TimelinePreviewResult> {
    try {
      await this.#prepareMedia(timeline);
      const sources = this.#sources.snapshot();
      const sourceDependencies = this.#sources.confirmedDigests();
      const resourceInputs = this.#resourceInputs;
      const locators = this.#locators;
      const mediaDependencies = this.#mediaDependencies;
      const result = await this.#compiler.compile(timelineCompilePayload(
        this.#host.boot, timeline, sources, resourceInputs,
      ));
      if (result.status === "error") throw new Error(result.message);
      const render = {
        timelineJson: result.package.timelineJson,
        timeline: JSON.parse(result.package.timelineJson) as TimelineDocument,
        fixedPackageManifestJson: result.package.fixedPackageManifestJson,
        resourceManifestJson: result.package.resourceManifestJson,
        resourceManifest: JSON.parse(result.package.resourceManifestJson) as ResourceManifest,
        verifiedBindingBundleJson: result.package.verifiedBindingBundleJson,
      };
      return { status: "ok", timeline, timelineJson: JSON.stringify(timeline), render,
        assets: locators, motion: { structures: [] },
        motionSourceDurations: result.package.motionSourceDurations,
        inputDependencies: { ...mediaDependencies, ...sourceDependencies },
        motionInstances: result.instances };
    } catch (error) {
      return { status: "error", diagnostics: [{ class: "compile", code: "studio-preview",
        message: error instanceof Error ? error.message : String(error) }] };
    }
  }

  async #prepareMedia(timeline: Timeline): Promise<void> {
    const key = mediaInputKey(timeline);
    if (key === null) {
      this.#mediaKey = null;
      this.#resourceInputs = [];
      this.#locators = [];
      this.#mediaDependencies = {};
      return;
    }
    if (key === this.#mediaKey) return;
    const preview = await this.#host.loadMediaFacts!({ timeline });
    if (preview.status === "error") {
      throw new Error(preview.diagnostics.map((item) => item.message).join("\n"));
    }
    this.#resourceInputs = preview.resources;
    this.#locators = preview.assets;
    this.#mediaDependencies = preview.inputDependencies;
    this.#mediaKey = key;
  }
}
