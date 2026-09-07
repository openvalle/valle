import type {
  CanonicalTimelineDocument,
  TimelineCompilerRuntime,
  TimelineCompilerRuntimeOptions,
} from "@valle/player-core";
import { createTimelineCompilerRuntime } from "@valle/player-core";
import type { Timeline } from "@valle/engine";
import type { ResourceManifest } from "@valle/engine/internal";
import type { VallePlayerElementOptions } from "@valle/player";

type PreviewPlayer = {
  load(options: VallePlayerElementOptions): Promise<void>;
};

export type TimelineWorkspaceRuntimeConfig = Omit<
  VallePlayerElementOptions,
  | "fixedPackageManifestJson"
  | "timelineJson"
  | "resourceManifestJson"
  | "verifiedBindingBundleJson"
> & {
  timeline: Timeline;
  timelineJson: string;
  render: {
    timeline: CanonicalTimelineDocument["timeline"];
    timelineJson: string;
  } & (
    | {
        fixedPackageManifestJson: string;
        resourceManifestJson: string;
        resourceManifest: ResourceManifest;
        verifiedBindingBundleJson: string;
      }
    | {
        fixedPackageManifestJson?: never;
        resourceManifestJson?: never;
        resourceManifest?: never;
        verifiedBindingBundleJson?: never;
      }
  );
  preview?: { status: "unavailable"; code: string };
};

export interface TimelineWorkspaceRuntime {
  previewAvailable: boolean;
  rendererMessage: string | null;
  compiledTimeline: CanonicalTimelineDocument;
  normalizeTimeline(timeline: Timeline): Timeline;
  timelineTimeFromFrames(frames: number, fps: Timeline["canvas"]["fps"]): number;
  timelineSourceTimeDeltaFromFrames(
    frames: number,
    fps: Timeline["canvas"]["fps"],
    rate?: number | null,
  ): number;
  compileTimeline(timeline: Timeline): CanonicalTimelineDocument;
}

type CompilerRuntimeFactory = (
  options: TimelineCompilerRuntimeOptions,
) => Promise<TimelineCompilerRuntime>;

/**
 * Bootstrap Timeline compilation independently from preview admission. A Project snapshot without
 * verified fulfillment still gets Rust/WASM canonicalization and view projection; it never opens
 * a product renderer with invented bindings.
 */
export async function initializeTimelineWorkspaceRuntime(
  config: TimelineWorkspaceRuntimeConfig,
  player: PreviewPlayer,
  createCompiler: CompilerRuntimeFactory = createTimelineCompilerRuntime,
): Promise<TimelineWorkspaceRuntime> {
  // Compilation is loaded even when a fixed-package preview is available. It is a read-only
  // projection from the Studio working copy; all edits remain on the sparse Timeline itself.
  const compiler = await createCompiler({
    runtimeAssets: config.runtimeAssets,
    runtimeBaseUrl: config.runtimeBaseUrl,
  });
  const compiled = compiler.compileTimeline(config.timeline);
  const hostedRender = compiler.canonicalizeTimelineDocument(config.render.timeline);
  if (hostedRender.timelineJson !== compiled.timelineJson) {
    throw new Error("Project render projection does not match the Timeline working copy");
  }
  const fixedPackageManifestJson = config.render.fixedPackageManifestJson;
  const resourceManifestJson = config.render.resourceManifestJson;
  const resourceManifest = config.render.resourceManifest;
  const verifiedBindingBundleJson = config.render.verifiedBindingBundleJson;
  const fixedFields = [
    fixedPackageManifestJson,
    resourceManifestJson,
    resourceManifest,
    verifiedBindingBundleJson,
  ];
  if (!fixedFields.every((field) => (field === undefined) === (fixedPackageManifestJson === undefined))) {
    throw new Error(
      "Timeline preview requires the complete fixed package manifest and members together",
    );
  }
  if (
    fixedFields.some((field) => typeof field === "string" && field.length === 0)
  ) {
    throw new Error("Timeline preview fixed package strings must not be empty");
  }
  if (
    fixedPackageManifestJson !== undefined
    && resourceManifestJson !== undefined
    && resourceManifest !== undefined
    && verifiedBindingBundleJson !== undefined
  ) {
    await player.load({
      fixedPackageManifestJson,
      timelineJson: config.render.timelineJson,
      resourceManifestJson,
      verifiedBindingBundleJson,
      assets: config.assets,
      assetBaseUrl: config.assetBaseUrl ?? "/assets/",
      proxyBase: config.proxyBase ?? null,
      runtimeAssets: config.runtimeAssets,
      runtimeBaseUrl: config.runtimeBaseUrl,
      pxScale: config.pxScale,
      gpu: config.gpu,
      audioContext: config.audioContext,
    });
    return {
      previewAvailable: true,
      rendererMessage: null,
      compiledTimeline: compiled,
      normalizeTimeline: (timeline) => compiler.normalizeTimeline(timeline),
      timelineTimeFromFrames: (frames, fps) => compiler.timelineTimeFromFrames(frames, fps),
      timelineSourceTimeDeltaFromFrames: (frames, fps, rate) => (
        compiler.timelineSourceTimeDeltaFromFrames(frames, fps, rate)
      ),
      compileTimeline: (timeline) => compiler.compileTimeline(timeline),
    };
  }
  return {
    previewAvailable: false,
    rendererMessage: previewUnavailableMessage(config.preview?.code),
    compiledTimeline: compiled,
    normalizeTimeline: (timeline) => compiler.normalizeTimeline(timeline),
    timelineTimeFromFrames: (frames, fps) => compiler.timelineTimeFromFrames(frames, fps),
    timelineSourceTimeDeltaFromFrames: (frames, fps, rate) => (
      compiler.timelineSourceTimeDeltaFromFrames(frames, fps, rate)
    ),
    compileTimeline: (timeline) => compiler.compileTimeline(timeline),
  };
}

function previewUnavailableMessage(code?: string): string {
  if (code === "verified_binding_bundle_unavailable" || code == null) {
    return "preview unavailable: verified binding fulfillment was not provided";
  }
  return `preview unavailable: ${code}`;
}
