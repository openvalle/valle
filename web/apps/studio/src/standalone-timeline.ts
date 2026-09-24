import type { Timeline } from "valle-engine";
import type { TimelineEdit, SaveReport } from "./host.ts";
import type { ResourceManifest, TimelineDocument } from "valle-engine/internal";
import {
  playerRuntimeAssetsFromStudioBoot,
  type StudioBoot,
  type StudioHost,
  type TimelineContext,
  type TimelinePreviewResult,
} from "./host.ts";
import type { StudioSourceEditor } from "./source-editor.ts";
import { StudioCompileClient, standaloneCompilePayload, type CompilePayload } from "./studio-compile.ts";
import type { StudioCompileResult } from "./studio-compile-worker.ts";
import { standaloneAssetsFromMotionContext, type StandaloneAssets } from "./standalone-assets.ts";

function wrapperShape(timeline: Timeline): string {
  const visual = timeline.tracks.visual?.map((track) => track.clips.map((clip) => {
    if (clip.kind === "motion") {
      const { props: _props, data: _data, ...shape } = clip;
      return shape;
    }
    return clip;
  }));
  return JSON.stringify({ canvas: timeline.canvas, resources: timeline.resources, visual,
    audio: timeline.tracks.audio, caption: timeline.tracks.caption, adjustment: timeline.tracks.adjustment });
}

function firstMotion(timeline: Timeline) {
  for (const track of timeline.tracks.visual ?? []) {
    for (const clip of track.clips) if (clip.kind === "motion") return clip;
  }
  return null;
}

function compiledPackage(result: Extract<StudioCompileResult, { status: "ok" }>) {
  return {
    timelineJson: result.package.timelineJson,
    timeline: JSON.parse(result.package.timelineJson) as TimelineDocument,
    fixedPackageManifestJson: result.package.fixedPackageManifestJson,
    resourceManifestJson: result.package.resourceManifestJson,
    resourceManifest: JSON.parse(result.package.resourceManifestJson) as ResourceManifest,
    verifiedBindingBundleJson: result.package.verifiedBindingBundleJson,
  };
}

/** Adapts a standalone source into the same author Timeline and package contract as other Studio entrances. */
export class StandaloneTimelineSession {
  readonly #boot: StudioBoot & { session: Extract<StudioBoot["session"], { kind: "motion-file" }> };
  readonly #sources: StudioSourceEditor;
  readonly #host: StudioHost;
  readonly #compiler: StudioCompileClient;
  #assetInputs: Promise<StandaloneAssets | undefined> | null = null;
  #lastWrapper: Timeline | null = null;
  #lastInstances: Extract<StudioCompileResult, { status: "ok" }>["instances"] = [];
  #generation = 0;
  #revision = 1;
  #target: string | null = null;
  #targetDigest: string | null = null;
  #ownedTimeline: Timeline | null = null;

  constructor(boot: StudioBoot, sources: StudioSourceEditor, host: StudioHost) {
    if (boot.session.kind !== "motion-file") throw new Error("Motion file session required");
    this.#boot = boot as StudioBoot & { session: Extract<StudioBoot["session"], { kind: "motion-file" }> };
    this.#sources = sources;
    this.#host = host;
    this.#compiler = new StudioCompileClient(boot);
  }

  close(): void { this.#compiler.close(); }

  invalidateAssetInputs(): void { this.#assetInputs = null; }

  get conversionBlockedReason(): string | null {
    return this.#boot.session.authorInputs?.extraFonts.length
      ? "Extra --font stacks must be replaced by a declared font asset before saving as Timeline"
      : null;
  }

  get target(): string | null { return this.#target; }

  async chooseTarget(): Promise<boolean> {
    if (this.conversionBlockedReason) throw new Error(this.conversionBlockedReason);
    if (this.#target) return true;
    const suggested = this.#boot.session.input.replace(/(?:\.motion)?\.[jt]sx?$/, "") + ".timeline.json";
    const dialog = document.getElementById("saveAsTimelineDialog") as HTMLDialogElement;
    const form = document.getElementById("saveAsTimelineForm") as HTMLFormElement;
    const input = document.getElementById("saveAsTimelinePath") as HTMLInputElement;
    const cancel = document.getElementById("saveAsTimelineCancel") as HTMLButtonElement;
    input.value = suggested;
    input.setCustomValidity("");
    return await new Promise<boolean>((resolve) => {
      const clearValidity = () => input.setCustomValidity("");
      const onSubmit = (event: SubmitEvent) => {
        event.preventDefault();
        const target = input.value.trim();
        if (!target.endsWith(".json")) {
          input.setCustomValidity("Timeline target must end in .json");
          input.reportValidity();
          return;
        }
        this.#target = target;
        dialog.close("save");
      };
      const onCancel = () => dialog.close("cancel");
      const onClose = () => {
        form.removeEventListener("submit", onSubmit);
        cancel.removeEventListener("click", onCancel);
        input.removeEventListener("input", clearValidity);
        resolve(dialog.returnValue === "save");
      };
      form.addEventListener("submit", onSubmit);
      cancel.addEventListener("click", onCancel);
      input.addEventListener("input", clearValidity);
      dialog.addEventListener("close", onClose, { once: true });
      dialog.showModal();
      input.focus();
    });
  }

  async saveTimeline(edit: TimelineEdit): Promise<SaveReport> {
    if (!this.#target || !this.#host.saveStandaloneTimeline) {
      throw new Error("Timeline save target is unavailable");
    }
    if (edit.baseRevision !== this.#revision) {
      return { outcome: "staleBase", revision: this.#revision };
    }
    const result = await this.#host.saveStandaloneTimeline({
      target: this.#target,
      baseDigest: this.#targetDigest,
      timeline: edit.timeline,
      expectedDependencies: { ...this.#boot.session.authorInputs?.inputDigests,
        ...(edit.expectedDependencies ?? this.#sources.confirmedDigests()) },
    });
    if (result.status === "conflict") return { outcome: "staleBase", revision: this.#revision + 1 };
    this.#target = result.target;
    this.#targetDigest = result.digest;
    this.#ownedTimeline = result.timeline;
    this.#revision++;
    return { outcome: "committed", revision: this.#revision };
  }

  async reloadSavedTimeline(): Promise<void> {
    if (!this.#target || !this.#host.loadSavedStandaloneTimeline) return;
    const saved = await this.#host.loadSavedStandaloneTimeline(this.#target);
    this.#target = saved.target;
    this.#targetDigest = saved.digest;
    this.#ownedTimeline = saved.timeline;
    this.#revision++;
  }

  async assets(): Promise<StandaloneAssets | undefined> {
    const specs = this.#boot.session.authorInputs?.assetSpecs ?? [];
    if (!specs.length) return undefined;
    this.#assetInputs ??= (async () => {
      if (!this.#host.loadMotion) throw new Error("Motion asset facts are unavailable from Host");
      return standaloneAssetsFromMotionContext(specs, await this.#host.loadMotion({}));
    })();
    try { return await this.#assetInputs; }
    catch (error) { this.#assetInputs = null; throw error; }
  }

  motionInstance(clipPath: string) {
    return this.#lastInstances.find((instance) => instance.clipPath === clipPath) ?? null;
  }

  async loadTimeline(): Promise<TimelineContext> {
    const result = await this.#compile(this.#ownedTimeline ?? undefined);
    const assets = await this.assets();
    const render = compiledPackage(result);
    const now = new Date().toISOString();
    return {
      timelineRevision: {
        revision: this.#revision, parentRevision: null, createdAt: now, actor: "studio",
        cause: { type: "genesis" }, intent: null,
      },
      timelineJson: JSON.stringify(result.authorTimeline),
      timeline: result.authorTimeline,
      motionSourceDurations: result.package.motionSourceDurations,
      inputDependencies: this.#sources.confirmedDigests(),
      render,
      motion: { structures: [] },
      assets: assets?.locators ?? [],
      runtimeAssets: playerRuntimeAssetsFromStudioBoot(this.#boot),
      assetBaseUrl: "/preview-assets/",
      generation: ++this.#generation,
      motionInstances: result.instances,
    };
  }

  async prepareTimelinePreview(timeline: Timeline): Promise<TimelinePreviewResult> {
    try {
      const result = await this.#compile(timeline);
      const assets = await this.assets();
      return {
        status: "ok",
        timelineJson: JSON.stringify(result.authorTimeline),
        timeline: result.authorTimeline,
        assets: assets?.locators ?? [], motion: { structures: [] },
        render: compiledPackage(result),
        motionSourceDurations: result.package.motionSourceDurations,
        generatedWrapper: this.#ownedTimeline === null
          && JSON.stringify(timeline) !== JSON.stringify(result.authorTimeline),
        inputDependencies: this.#sources.confirmedDigests(),
        motionInstances: result.instances,
      };
    } catch (error) {
      return { status: "error", diagnostics: [{ class: "compile", code: "studio-preview",
        message: error instanceof Error ? error.message : String(error) }] };
    }
  }

  async #compile(draft?: Timeline): Promise<Extract<StudioCompileResult, { status: "ok" }>> {
    const payload: CompilePayload = standaloneCompilePayload(this.#boot, this.#sources.snapshot(),
      undefined, undefined, await this.assets());
    if (draft && !this.#ownedTimeline && this.#lastWrapper && wrapperShape(draft) === wrapperShape(this.#lastWrapper)) {
      const clip = firstMotion(draft);
      if (clip && payload.standalone) {
        payload.standalone.props = clip.props ?? this.#boot.session.authorInputs?.props;
        const lastData = firstMotion(this.#lastWrapper)?.data ?? {};
        if (clip.data && JSON.stringify(clip.data) !== JSON.stringify(lastData)) {
          payload.standalone.data = clip.data;
          payload.instances[0]!.options = { data: { source: "studio:inline", value: clip.data } };
        }
      }
    } else if (draft) {
      payload.authorTimeline = draft;
      delete payload.standalone;
      const original = payload.instances[0];
      payload.instances = [];
      if (!original) throw new Error("Motion source closure is missing");
      for (const [trackIndex, track] of (draft.tracks.visual ?? []).entries()) {
        for (const [clipIndex, clip] of track.clips.entries()) {
          if (clip.kind !== "motion") continue;
          payload.instances.push({ ...original,
            clipPath: `/tracks/visual/${trackIndex}/clips/${clipIndex}`,
            options: clip.data ? { ...original.options, data: { source: `timeline:${trackIndex}:${clipIndex}`, value: clip.data } }
              : original.options,
          });
        }
      }
    }
    const result = await this.#compiler.compile(payload);
    if (result.status === "error") throw new Error(result.message);
    this.#lastInstances = result.instances;
    if (!draft || (!this.#ownedTimeline && this.#lastWrapper && wrapperShape(draft) === wrapperShape(this.#lastWrapper))) {
      this.#lastWrapper = result.authorTimeline;
    }
    return result;
  }
}
