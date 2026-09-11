import type { Timeline } from "@valle/engine";
import type {
  TimelineDocumentView,
  VallePlayerElement,
  VallePlayerElementOptions,
} from "@valle/player";
import type { TimelineDocument } from "@valle/engine/internal";

import {
  projectMotionContextFromAdmittedPreview,
  type StudioHost,
  type TimelineContext,
} from "./host.ts";
import type { GoodMotionContext } from "./motion-preview.ts";
import type { ProjectMotionSession } from "./motion-workspace.ts";
import {
  motionContextWithTimelineFrames,
  preparedMotionProps,
  type ProjectMotionEdit,
} from "./project-motion-edit.ts";
import type { ValleStudioApp } from "./studio-shell.ts";
import type {
  InspectorSectionView,
  StudioHeaderMeta,
  StudioInspector,
  StudioInspectorIntent,
  StudioMetaChip,
  StudioProjectControls,
  StudioProjectIntent,
  StudioTimeline as StudioTimelineElement,
  StudioTransport,
  StudioTransportIntent,
  TimelineViewModel,
} from "./timeline-components.ts";
import { canvasDragPosition, crossedCanvasDragThreshold } from "./timeline-canvas-drag.ts";
import { TimelineEditQueue } from "./timeline-edit-queue.ts";
import { TimelineSaveQueue } from "./timeline-save-queue.ts";
import {
  deleteTimelineClip,
  editTimelineClip,
  editTimelineMotionFrames,
  moveTimelineVisualClipBefore,
  setTimelineMotionProp,
  setTimelineClipDurationFrames,
  setTimelineSourceStartFrames,
  trimTimelineClipFrames,
} from "./timeline-edit.ts";
import { initializeTimelineWorkspaceRuntime } from "./timeline-workspace-runtime.ts";
import {
  assertReloadedTimelineSave,
  captureTimelineSaveSnapshot,
} from "./timeline-save-snapshot.ts";
import { hitAtDisplayPoint, topHitForClip } from "./timeline-selection.ts";
import {
  findProjectedItem,
  projectTimelineSequences,
  type ProjectedSequenceItem,
  type AudioItem,
  type CaptionItem,
  type TimelineSequenceItem,
  type VisualItem,
} from "./timeline-sequence.ts";

type PlayerOptions = VallePlayerElementOptions;
type Player = VallePlayerElement;
type PlayerHitRect = Player["hitRects"][number];
type MotionProjection = Record<string, unknown> & { structures: unknown[] };
type VisualClip = Extract<VisualItem, { type: "clip" }>;
type AudioClip = Extract<AudioItem, { type: "clip" }>;
type Caption = Extract<CaptionItem, { type: "clip" }>;

type StudioConfig = Omit<
  Partial<PlayerOptions>,
  | "fixedPackageManifestJson"
  | "timelineJson"
  | "resourceManifestJson"
  | "verifiedBindingBundleJson"
> & TimelineContext & {
  runtimeAssets: PlayerOptions["runtimeAssets"];
  assetBaseUrl?: string;
  generation?: number;
  initialTimeS?: number;
  captures?: Array<{ timeS: number; sampleId: string }>;
};

function fixedPackageReplacement(config: StudioConfig) {
  const fixedPackageManifestJson = config.render.fixedPackageManifestJson;
  const resourceManifestJson = config.render.resourceManifestJson;
  const resourceManifest = config.render.resourceManifest;
  const verifiedBindingBundleJson = config.render.verifiedBindingBundleJson;
  if (
    fixedPackageManifestJson === undefined
    || resourceManifestJson === undefined
    || resourceManifest === undefined
    || verifiedBindingBundleJson === undefined
  ) return null;
  return {
    fixedPackageManifestJson,
    timelineJson: config.render.timelineJson,
    resourceManifestJson,
    verifiedBindingBundleJson,
    assets: config.assets,
  };
}

interface CanvasDragState {
  timelinePath: string;
  pointerId: number;
  startX: number;
  startY: number;
  baseX: number;
  baseY: number;
  moved: boolean;
  lastX: number;
  lastY: number;
}

interface TimelineTrimState {
  timelinePath: string;
  edge: "start" | "end";
  pointerId: number;
  startX: number;
}

declare global {
  var valleStudioPlayer: Player | undefined;
}

export interface TimelineWorkspaceOptions {
  openMotion?(session: ProjectMotionSession): Promise<void>;
}

function requiredElement<T extends HTMLElement = HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing required Studio element #${id}`);
  return element as T;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.stack ?? error.message : String(error ?? "unknown error");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isMotionProjection(value: unknown): value is MotionProjection {
  return isRecord(value) && Array.isArray(value.structures);
}

async function postFailure(message: unknown): Promise<void> {
  await fetch("/result", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ status: "error", message: String(message) }),
  });
}

function showFatalError(message: unknown): void {
  requiredElement("errtext").textContent = String(message ?? "unknown error");
  requiredElement("errbox").hidden = false;
  requiredElement("loading").hidden = true;
}

function formatSeconds(seconds: number): string {
  const value = Math.max(0, Number.isFinite(seconds) ? seconds : 0);
  const minutes = Math.floor(value / 60);
  const rest = value - minutes * 60;
  return `${minutes}:${rest.toFixed(3).padStart(6, "0")}`;
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return btoa(binary);
}

function sourceKind(item: TimelineSequenceItem): string {
  if ("type" in item && item.type === "clip" && "layer" in item) return item.source.type;
  if ("type" in item && item.type === "clip" && "source" in item) return "audio";
  if ("type" in item && item.type === "clip" && "runs" in item) return "caption";
  if ("effect" in item) return "adjustment";
  return "type" in item ? item.type : "item";
}

function itemLabel(item: TimelineSequenceItem): string {
  if ("type" in item && item.type === "clip" && "layer" in item) {
    if (item.source.type === "motion") return item.source.component;
    if (item.source.type === "solid") return item.source.color;
    return item.source.resource;
  }
  if ("type" in item && item.type === "clip" && "source" in item) return item.source.resource;
  if ("type" in item && item.type === "clip" && "runs" in item) {
    return item.runs.map((run: { text: string }) => run.text).join("");
  }
  if ("effect" in item) return item.effect.type;
  throw new Error("unsupported Timeline item");
}

function visualClip(item: TimelineSequenceItem): VisualClip | null {
  return "type" in item && item.type === "clip" && "layer" in item ? item : null;
}

function audioClip(item: TimelineSequenceItem): AudioClip | null {
  return "type" in item && item.type === "clip" && "source" in item && !("layer" in item)
    ? item
    : null;
}

function captionItem(item: TimelineSequenceItem): Caption | null {
  return "type" in item && item.type === "clip" && "runs" in item ? item : null;
}

function constantVec2(
  parameter: VisualClip["layer"]["transform"]["position"],
  label: string,
): [number, number] {
  if (parameter.type !== "constant") throw new Error(`${label} is curve-driven and cannot be directly edited`);
  return parameter.value;
}

async function main(options: TimelineWorkspaceOptions = {}): Promise<void> {
  const studioHost: StudioHost | undefined = globalThis.valleStudioHost;
  if (!studioHost?.loadTimeline) throw new Error("Studio Timeline host is unavailable");
  const loadTimeline = studioHost.loadTimeline.bind(studioHost);
  const saveTimeline = studioHost.saveTimeline?.bind(studioHost);
  const boot = studioHost.boot;
  if (boot.session.kind === "motion-file") throw new Error("Motion session cannot open Timeline workspace");

  const shell = requiredElement<ValleStudioApp>("studioShell");
  const meta = requiredElement<StudioHeaderMeta>("meta");
  const transport = requiredElement<StudioTransport>("studioTransport");
  const timelineView = requiredElement<StudioTimelineElement>("tracks");
  const inspector = requiredElement<StudioInspector>("inspectorBody");
  const projectControls = requiredElement<StudioProjectControls>("projectControls");
  const player = requiredElement<VallePlayerElement>("studioPlayer");
  await Promise.all([
    meta.updateComplete,
    transport.updateComplete,
    timelineView.updateComplete,
    inspector.updateComplete,
    projectControls.updateComplete,
    player.updateComplete,
  ]);

  let config = await loadTimeline() as StudioConfig;
  let workingCopy: Timeline = structuredClone(config.timeline);
  let motionProjection: MotionProjection | null = isMotionProjection(config.motion)
    ? structuredClone(config.motion)
    : null;
  let baseRevision = config.timelineRevision.revision;
  let selectedItemId: string | null = shell.shellState.selectedClipId;
  let dirty = shell.shellState.dirty;
  let conflictMessage: string | null = null;
  let pixelsPerSecond = 40;
  let playhead: HTMLElement | null = null;
  let canvasDrag: CanvasDragState | null = null;
  let sequenceDragPath: string | null = null;
  let timelineTrim: TimelineTrimState | null = null;
  const editQueue = new TimelineEditQueue();
  const projectMode = boot.session.kind === "project" && boot.capabilities.saveTimeline;
  const canvas = workingCopy.canvas;
  player.style.aspectRatio = `${canvas.width} / ${canvas.height}`;

  const workspaceRuntime = await initializeTimelineWorkspaceRuntime(
    { ...config, runtimeBaseUrl: config.runtimeBaseUrl ?? location.href },
    player,
  );
  const compileTimeline = workspaceRuntime.compileTimeline;
  const normalizeTimeline = workspaceRuntime.normalizeTimeline;
  const timelineTimeFromFrames = workspaceRuntime.timelineTimeFromFrames;
  const timelineSourceTimeDeltaFromFrames = workspaceRuntime.timelineSourceTimeDeltaFromFrames;
  let compiledCopy = workspaceRuntime.compiledTimeline;
  let playerLoaded = workspaceRuntime.previewAvailable;
  let previewAvailable = workspaceRuntime.previewAvailable;
  let rendererMessage: string | null = workspaceRuntime.rendererMessage;
  if (!previewAvailable && rendererMessage) player.loadingLabel = rendererMessage;
  let documentView: TimelineDocumentView = compiledCopy.view;
  let fps = documentView.canvas.framesPerSecond;
  let editingTimeS = Math.max(
    0,
    Math.min(config.initialTimeS ?? 0, documentView.canvas.durationSeconds),
  );
  const loadedPlayerCanvas = player.canvas;
  if (!loadedPlayerCanvas) throw new Error("Studio player has no canvas");
  const playerCanvas: HTMLCanvasElement = loadedPlayerCanvas;
  globalThis.valleStudioPlayer = previewAvailable ? player : undefined;

  function currentTime(): number {
    return previewAvailable ? player.currentTime() : editingTimeS;
  }

  function isPlaying(): boolean {
    return previewAvailable && player.playing;
  }

  async function seek(timeS: number): Promise<void> {
    if (previewAvailable) {
      await player.seek(timeS);
      return;
    }
    editingTimeS = Math.max(0, Math.min(timeS, documentView.canvas.durationSeconds));
  }

  async function replacePreview(next: StudioConfig): Promise<boolean> {
    const replacement = fixedPackageReplacement(next);
    if (!replacement) {
      if (playerLoaded) player.pause();
      previewAvailable = false;
      globalThis.valleStudioPlayer = undefined;
      rendererMessage = "preview unavailable: verified binding fulfillment was not provided";
      player.loadingLabel = rendererMessage;
      return false;
    }
    if (playerLoaded) {
      await player.replaceRenderPackage(replacement);
    } else {
      const runtime = await initializeTimelineWorkspaceRuntime(
        { ...next, runtimeBaseUrl: next.runtimeBaseUrl ?? location.href },
        player,
      );
      playerLoaded = runtime.previewAvailable;
    }
    previewAvailable = true;
    globalThis.valleStudioPlayer = player;
    return true;
  }

  function renderMeta(): void {
    const currentCanvas = workingCopy.canvas;
    const chips: StudioMetaChip[] = [
      { text: `${currentCanvas.width}×${currentCanvas.height}`, strong: true },
      { text: `${fps.toFixed(Number.isInteger(fps) ? 0 : 3)} fps`, strong: true },
      { text: formatSeconds(documentView.canvas.durationSeconds), strong: true },
    ];
    if (dirty) chips.push({ text: "edited", tone: "live" });
    if (conflictMessage) chips.push({ text: conflictMessage, tone: "warning" });
    if (rendererMessage) chips.push({ text: `renderer: ${rendererMessage}`, tone: "warning" });
    meta.chips = chips;
    projectControls.sync({ visible: projectMode, dirty, conflictMessage });
  }

  function markDirty(): void {
    if (!dirty) shell.dispatchIntent({ type: "dirty", value: true });
    dirty = true;
    saveQueue.noteEdit();
    renderMeta();
  }

  function setConflict(message: string | null): void {
    conflictMessage = message;
    shell.dispatchIntent({ type: "conflict", value: message !== null });
    renderMeta();
  }

  function sequenceView(): TimelineViewModel {
    const duration = Math.max(0.001, documentView.canvas.durationSeconds);
    const laneWidth = Math.max(1, Math.ceil(duration * pixelsPerSecond));
    const labelWidth = 140;
    const tickSeconds = pixelsPerSecond >= 120 ? 1 : pixelsPerSecond >= 30 ? 5 : 10;
    const ticks: TimelineViewModel["ticks"][number][] = [];
    for (let second = 0; second <= duration; second += tickSeconds) {
      ticks.push({ leftPx: labelWidth + second * pixelsPerSecond, label: formatSeconds(second) });
    }
    const tracks = projectTimelineSequences(compiledCopy.timeline, documentView).map((track) => ({
      id: track.id,
      kind: track.band,
      clips: track.items.map((entry) => {
        const durationBearing = entry.advancesCursor || entry.band === "adjustment";
        return {
          id: entry.item.id,
          kind: sourceKind(entry.item),
          leftPx: Math.max(0, entry.startSeconds * pixelsPerSecond),
          widthPx: durationBearing
            ? Math.max(3, entry.durationSeconds * pixelsPerSecond)
            : 3,
          title: durationBearing
            ? `${entry.item.id} · ${formatSeconds(entry.startSeconds)} · ${entry.item.duration}`
            : `${entry.item.id} · cut ${formatSeconds(entry.startSeconds)} · nominal ${entry.item.duration}`,
          label: itemLabel(entry.item),
          selected: selectedItemId === entry.item.id,
          timingEditable: durationBearing && entry.timelinePath !== null,
        };
      }),
    }));
    return {
      widthPx: labelWidth + laneWidth,
      laneWidthPx: laneWidth,
      playheadLeftPx: labelWidth + currentTime() * pixelsPerSecond,
      ticks,
      tracks,
    };
  }

  function renderTimeline(): void {
    timelineView.renderTimeline(sequenceView());
    playhead = timelineView.querySelector<HTMLElement>("#tlPlayhead");
    positionPlayhead();
  }

  function positionPlayhead(): void {
    if (playhead) playhead.style.left = `${140 + currentTime() * pixelsPerSecond}px`;
  }

  function selectedProjection(): ProjectedSequenceItem | null {
    return selectedItemId
      ? findProjectedItem(compiledCopy.timeline, documentView, selectedItemId)
      : null;
  }

  function renderInspector(): void {
    const projected = selectedProjection();
    if (!projected) {
      inspector.renderInspector(null);
      return;
    }
    const item = projected.item;
    const sections: InspectorSectionView[] = [{
      rows: [
        { kind: "value", label: "id", value: item.id },
        { kind: "value", label: "band", value: projected.band },
        { kind: "value", label: "type", value: sourceKind(item) },
        { kind: "value", label: "Timeline path", value: projected.timelinePath ?? "generated" },
        { kind: "value", label: "derived start", value: formatSeconds(projected.startSeconds) },
        ...(projected.timelinePath
          ? [{ kind: "number" as const, key: "timing:durationFrames", label: "duration (frames)", value: projected.durationFrames, min: 1, step: 1 }]
          : [{ kind: "value" as const, label: "duration (frames)", value: String(projected.durationFrames) }]),
        { kind: "value", label: "duration (exact)", value: String(item.duration) },
      ],
    }];
    let motionSource: string | undefined;
    const visual = visualClip(item);
    if (visual) {
      const rows: InspectorSectionView["rows"][number][] = [
        { kind: "value", label: "source", value: visual.source.type },
      ];
      if (visual.source.type === "solid") {
        rows.push({ kind: "textarea", key: "source:color", label: "color", value: visual.source.color });
      }
      if (visual.source.type === "video" || visual.source.type === "lottie" || visual.source.type === "motion") {
        rows.push(
          { kind: "number", key: "timing:sourceStartFrames", label: "source start (frames)", value: projected.sourceStartFrame ?? 0, min: 0, step: 1 },
          { kind: "value", label: "source start (exact)", value: visual.source.sourceStart },
        );
      }
      if (visual.layer.opacity.type === "constant") {
        rows.push({ kind: "number", key: "layer:opacity", label: "opacity", value: visual.layer.opacity.value, min: 0, max: 1, step: 0.01 });
      } else {
        rows.push({ kind: "value", label: "opacity", value: "curve-driven (read only)" });
      }
      if (visual.source.type === "video") {
        const gain = visual.source.gain;
        if (!gain || gain.type === "constant") rows.push({ kind: "number", key: "source:gain", label: "volume", value: gain?.value ?? 1, min: 0, step: 0.1 });
        else rows.push({ kind: "value", label: "volume", value: "curve-driven (read only)" });
      }
      const { position, scale, rotation, size } = visual.layer.transform;
      if (size?.type === "constant") {
        rows.push(
          { kind: "number", key: "layer:size-x", label: "width (px)", value: size.value[0], min: 1, step: 1 },
          { kind: "number", key: "layer:size-y", label: "height (px)", value: size.value[1], min: 1, step: 1 },
        );
      } else if (size) rows.push({ kind: "value", label: "size", value: "curve-driven (read only)" });
      if (position.type === "constant") {
        rows.push(
          { kind: "number", key: "layer:position-x", label: "position x", value: position.value[0], step: 0.001 },
          { kind: "number", key: "layer:position-y", label: "position y", value: position.value[1], step: 0.001 },
        );
      } else rows.push({ kind: "value", label: "position", value: "curve-driven (read only)" });
      if (scale.type === "constant") {
        rows.push(
          { kind: "number", key: "layer:scale-x", label: "scale x", value: scale.value[0], step: 0.01 },
          { kind: "number", key: "layer:scale-y", label: "scale y", value: scale.value[1], step: 0.01 },
        );
      } else rows.push({ kind: "value", label: "scale", value: "curve-driven (read only)" });
      if (rotation.type === "constant") {
        rows.push({ kind: "number", key: "layer:rotation", label: "rotation (degrees)", value: rotation.value * 180 / Math.PI, step: 1 });
      } else rows.push({ kind: "value", label: "rotation", value: "curve-driven (read only)" });
      if (visual.source.type === "motion") motionSource = visual.source.component;
      sections.push({ title: "visual clip", rows });
    }
    const audio = audioClip(item);
    if (audio) {
      const rows: InspectorSectionView["rows"][number][] = [
        { kind: "value", label: "resource", value: audio.source.resource },
        { kind: "number", key: "timing:sourceStartFrames", label: "source start (frames)", value: projected.sourceStartFrame ?? 0, min: 0, step: 1 },
        { kind: "value", label: "source start (exact)", value: audio.source.sourceStart },
      ];
      if (audio.gain.type === "constant") rows.push({ kind: "number", key: "audio:gain", label: "gain", value: audio.gain.value, min: 0, step: 0.01 });
      else rows.push({ kind: "value", label: "gain", value: "curve-driven (read only)" });
      if (audio.pan.type === "constant") rows.push({ kind: "number", key: "audio:pan", label: "pan", value: audio.pan.value, min: -1, max: 1, step: 0.01 });
      else rows.push({ kind: "value", label: "pan", value: "curve-driven (read only)" });
      sections.push({ title: "audio clip", rows });
    }
    const caption = captionItem(item);
    if (caption) {
      sections.push({ title: "caption", rows: [
        ...caption.runs.map((run: { id: string; text: string }, runIndex: number) => ({
          kind: "textarea" as const,
          key: `caption:run:${runIndex}`,
          label: run.id,
          value: run.text,
          primaryText: true,
        })),
        { kind: "textarea", key: "caption:color", label: "color", value: caption.style.color },
        { kind: "number", key: "caption:font-size", label: "font size", value: caption.style.fontSize, min: 1, step: 1 },
      ] });
    }
    inspector.renderInspector({
      clipId: item.id,
      sections,
      rawJson: JSON.stringify(item, null, 2),
      motionSource,
    });
  }

  function setSelected(itemId: string | null): void {
    selectedItemId = itemId && findProjectedItem(compiledCopy.timeline, documentView, itemId)
      ? itemId
      : null;
    shell.dispatchIntent({ type: "select", clipId: selectedItemId });
    renderTimeline();
    renderInspector();
    positionSelectionBox();
  }

  function renderWorkingCopy(): void {
    renderTimeline();
    renderInspector();
    renderMeta();
    transport.configure(documentView.canvas.durationSeconds, fps);
    syncTransport();
  }

  async function commitWorkingCopy(next: Timeline): Promise<void> {
    // The working copy is editing truth. Renderer supersede/failure is reported separately and
    // never rolls it back.
    const normalized = normalizeTimeline(next);
    const compiled = compileTimeline(normalized);
    workingCopy = structuredClone(normalized);
    compiledCopy = compiled;
    documentView = compiled.view;
    fps = documentView.canvas.framesPerSecond;
    player.style.aspectRatio = `${documentView.canvas.width} / ${documentView.canvas.height}`;
    selectedItemId = selectedItemId && findProjectedItem(compiledCopy.timeline, documentView, selectedItemId)
      ? selectedItemId
      : null;
    markDirty();
    renderWorkingCopy();
    rendererMessage = "preview remains pinned to the committed revision until save";
    renderMeta();
  }

  function queueWorkingCopy(next: () => Timeline): Promise<void> {
    return editQueue.enqueue(
      () => commitWorkingCopy(next()),
      (error) => {
        rendererMessage = error instanceof Error ? error.message : String(error);
        renderMeta();
      },
    );
  }

  function editItem(itemId: string, key: string, value: string | number): Promise<void> {
    return queueWorkingCopy(() => {
      const projected = findProjectedItem(compiledCopy.timeline, documentView, itemId);
      if (!projected) throw new Error(`Timeline clip '${itemId}' does not exist`);
      if (!projected.timelinePath) {
        throw new Error(`Timeline item '${itemId}' is compiler-generated and cannot be edited`);
      }
      if (key === "timing:durationFrames" || key === "timing:sourceStartFrames") {
        const frames = Number(value);
        return key === "timing:durationFrames"
          ? setTimelineClipDurationFrames(
            workingCopy,
            projected.timelinePath,
            frames,
            timelineTimeFromFrames,
          )
          : setTimelineSourceStartFrames(
            workingCopy,
            projected.timelinePath,
            frames,
            timelineTimeFromFrames,
          );
      }
      const item = projected.item;
      const visual = visualClip(item);
      const audio = audioClip(item);
      const caption = captionItem(item);
      return editTimelineClip(workingCopy, projected.timelinePath, (target) => {
        if (key === "source:color" && visual?.source.type === "solid") {
          if (target.band !== "visual" || target.clip.kind !== "solid") {
            throw new Error("selected visual source is not a solid clip");
          }
          target.clip.color = String(value);
          return;
        }
        if (visual?.source.type === "video" && key === "source:gain") {
          if (target.band !== "visual" || target.clip.kind !== "video") throw new Error("selected source is not a video");
          if (visual.source.gain && visual.source.gain.type !== "constant") throw new Error("curve-driven gain is not directly editable");
          target.clip.gain = Number(value);
          return;
        }
        if (visual && key === "layer:opacity") {
          if (target.band !== "visual") throw new Error("selected item is not a visual clip");
          if (visual.layer.opacity.type !== "constant") throw new Error("curve-driven opacity is not directly editable");
          target.clip.opacity = Number(value);
          return;
        }
        if (visual && key.startsWith("layer:")) {
          if (target.band !== "visual") throw new Error("selected item is not a visual clip");
          const transform = visual.layer.transform;
          if (key === "layer:rotation") {
            if (transform.rotation.type !== "constant") throw new Error("curve-driven rotation is not directly editable");
            target.clip.rotation = Number(value);
            return;
          }
          const field = key.includes("position") ? "position" : key.includes("size") ? "size" : "scale";
          const transformParam = transform[field];
          if (!transformParam || transformParam.type !== "constant") throw new Error("curve-driven vector is not directly editable");
          const authored = target.clip[field];
          const current: [number, number] = Array.isArray(authored)
            ? [Number(authored[0]), Number(authored[1])]
            : [transformParam.value[0], transformParam.value[1]];
          current[key.endsWith("-x") ? 0 : 1] = Number(value);
          target.clip[field] = current;
          return;
        }
        if (audio && (key === "audio:gain" || key === "audio:pan")) {
          if (target.band !== "audio") throw new Error("selected item is not an audio clip");
          const parameter = key === "audio:gain" ? audio.gain : audio.pan;
          if (parameter.type !== "constant") throw new Error(`curve-driven ${key.slice(6)} is not directly editable`);
          if (key === "audio:gain") target.clip.gain = Number(value);
          else target.clip.pan = Number(value);
          return;
        }
        if (caption && key.startsWith("caption:run:")) {
          if (target.band !== "caption") throw new Error("selected item is not a caption clip");
          const runToken = key.slice("caption:run:".length);
          if (!/^(0|[1-9]\d*)$/.test(runToken)) {
            throw new Error(`caption run index '${runToken}' is invalid`);
          }
          const runIndex = Number(runToken);
          if (typeof target.clip.text === "string") {
            if (runIndex !== 0) throw new Error(`caption run ${runIndex} does not exist`);
            target.clip.text = String(value);
            return;
          }
          const runs = target.clip.runs ?? [];
          if (!runs[runIndex]) throw new Error(`caption run ${runIndex} does not exist`);
          runs[runIndex]!.text = String(value);
          return;
        }
        if (caption && (key === "caption:color" || key === "caption:font-size")) {
          if (target.band !== "caption") throw new Error("selected item is not a caption clip");
          const style = target.track.style;
          if (key === "caption:color") {
            style.color = String(value);
          } else {
            style.fontSize = Number(value);
          }
          return;
        }
        throw new Error(`Studio cannot edit '${key}' on ${sourceKind(item)}`);
      });
    });
  }

  function deleteItem(itemId: string): Promise<void> {
    return queueWorkingCopy(() => {
      const projected = findProjectedItem(compiledCopy.timeline, documentView, itemId);
      if (!projected?.timelinePath) {
        throw new Error(`Timeline item '${itemId}' is compiler-generated and cannot be deleted`);
      }
      return deleteTimelineClip(workingCopy, projected.timelinePath);
    })
      .then(() => setSelected(null));
  }

  function syncTransport(): void {
    transport.sync(currentTime(), documentView.canvas.durationSeconds, fps, isPlaying());
    positionPlayhead();
    positionSelectionBox();
  }

  let animationFrame: number | null = null;
  function startTransportPoll(): void {
    if (animationFrame !== null) return;
    const tick = () => {
      syncTransport();
      if (isPlaying()) animationFrame = requestAnimationFrame(tick);
      else animationFrame = null;
    };
    animationFrame = requestAnimationFrame(tick);
  }

  transport.addEventListener("studio-transport-intent", (event) => {
    const intent = (event as CustomEvent<StudioTransportIntent>).detail;
    if (intent.type === "toggle-play") {
      if (!previewAvailable) {
        rendererMessage = "preview unavailable: transport is disabled";
        renderMeta();
      } else if (player.playing) {
        player.pause();
        syncTransport();
      } else {
        void player.play().then(startTransportPoll);
      }
    } else if (intent.type === "pause") {
      if (previewAvailable) player.pause();
    } else {
      void seek(intent.timeS).then(syncTransport);
    }
  });
  player.addEventListener("time", syncTransport);

  timelineView.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const itemElement = target?.closest<HTMLElement>(".tl-clip");
    if (itemElement?.dataset.clipId) {
      setSelected(itemElement.dataset.clipId);
      return;
    }
    const ruler = target?.closest<HTMLElement>(".tl-ruler");
    if (ruler) {
      const rect = ruler.getBoundingClientRect();
      void seek(Math.max(0, (event.clientX - rect.left - 140) / pixelsPerSecond)).then(syncTransport);
    }
  });

  timelineView.addEventListener("pointerdown", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const handle = target?.closest<HTMLElement>(".tl-trim");
    if (handle) {
      event.preventDefault();
      const itemId = handle.closest<HTMLElement>(".tl-clip")?.dataset.clipId;
      if (!itemId) return;
      const projected = findProjectedItem(compiledCopy.timeline, documentView, itemId);
      if (!projected?.timelinePath) return;
      timelineTrim = {
        timelinePath: projected.timelinePath,
        edge: handle.classList.contains("w") ? "start" : "end",
        pointerId: event.pointerId,
        startX: event.clientX,
      };
      sequenceDragPath = null;
      timelineView.setPointerCapture(event.pointerId);
      return;
    }
    const block = target?.closest<HTMLElement>(".tl-clip");
    const itemId = block?.dataset.clipId;
    const projected = itemId
      ? findProjectedItem(compiledCopy.timeline, documentView, itemId)
      : null;
    if (projected?.band === "visual" && projected.timelinePath && visualClip(projected.item)) {
      sequenceDragPath = projected.timelinePath;
    }
  });
  timelineView.addEventListener("pointerup", (event) => {
    const trim = timelineTrim;
    timelineTrim = null;
    if (trim && event.pointerId === trim.pointerId) {
      if (timelineView.hasPointerCapture(event.pointerId)) {
        timelineView.releasePointerCapture(event.pointerId);
      }
      // Gesture-to-frame selection is presentation logic. Rust converts the integral delta with
      // the exact authored frame rate, then normalizes the complete sparse working copy.
      const pixelsPerFrame = pixelsPerSecond / documentView.canvas.framesPerSecond;
      const deltaFrames = Math.round((event.clientX - trim.startX) / pixelsPerFrame);
      if (deltaFrames !== 0) {
        void queueWorkingCopy(() => trimTimelineClipFrames(
          workingCopy,
          trim.timelinePath,
          trim.edge,
          deltaFrames,
          timelineTimeFromFrames,
          timelineSourceTimeDeltaFromFrames,
        ));
      }
      return;
    }
    const source = sequenceDragPath;
    sequenceDragPath = null;
    const target = event.target instanceof Element
      ? event.target.closest<HTMLElement>(".tl-clip")?.dataset.clipId
      : undefined;
    if (!source || !target || source === target) return;
    const targetProjection = findProjectedItem(compiledCopy.timeline, documentView, target);
    if (targetProjection?.band !== "visual" || !targetProjection.timelinePath
      || !visualClip(targetProjection.item)) return;
    const targetPath = targetProjection.timelinePath;
    void queueWorkingCopy(() => moveTimelineVisualClipBefore(
      workingCopy,
      source,
      targetPath,
    )).then(() => setSelected(null));
  });
  timelineView.addEventListener("pointercancel", (event) => {
    if (timelineTrim?.pointerId === event.pointerId) timelineTrim = null;
  });

  const selectionBox = requiredElement<HTMLElement>("selBox");
  const stageWrap = requiredElement<HTMLElement>("stageWrap");
  function selectedHit(): PlayerHitRect | null {
    return previewAvailable && selectedItemId
      ? topHitForClip(player.hitRects ?? [], selectedItemId)
      : null;
  }
  function positionSelectionBox(): void {
    const hit = selectedHit();
    if (!hit) {
      selectionBox.hidden = true;
      return;
    }
    const canvasRect = playerCanvas.getBoundingClientRect();
    const stageRect = stageWrap.getBoundingClientRect();
    const scaleX = canvasRect.width / Math.max(1, playerCanvas.width);
    const scaleY = canvasRect.height / Math.max(1, playerCanvas.height);
    selectionBox.style.left = `${canvasRect.left - stageRect.left + hit.rect.x * scaleX}px`;
    selectionBox.style.top = `${canvasRect.top - stageRect.top + hit.rect.y * scaleY}px`;
    selectionBox.style.width = `${Math.max(0, hit.rect.w * scaleX)}px`;
    selectionBox.style.height = `${Math.max(0, hit.rect.h * scaleY)}px`;
    selectionBox.hidden = false;
  }

  playerCanvas.addEventListener("pointerdown", (event) => {
    if (!previewAvailable || !event.isPrimary) return;
    const rect = playerCanvas.getBoundingClientRect();
    const hit = hitAtDisplayPoint(
      player.hitRects ?? [],
      event.clientX,
      event.clientY,
      rect,
      playerCanvas.width,
      playerCanvas.height,
    );
    if (!hit?.clipId) {
      setSelected(null);
      return;
    }
    setSelected(hit.clipId);
    const projected = findProjectedItem(compiledCopy.timeline, documentView, hit.clipId);
    const clip = projected && visualClip(projected.item);
    if (!clip || !projected?.timelinePath) return;
    try {
      const position = constantVec2(clip.layer.transform.position, "position");
      canvasDrag = {
        timelinePath: projected.timelinePath,
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        baseX: position[0],
        baseY: position[1],
        moved: false,
        lastX: position[0],
        lastY: position[1],
      };
      playerCanvas.setPointerCapture(event.pointerId);
    } catch (error) {
      rendererMessage = error instanceof Error ? error.message : String(error);
      renderMeta();
    }
  });
  playerCanvas.addEventListener("pointermove", (event) => {
    if (!canvasDrag || event.pointerId !== canvasDrag.pointerId) return;
    if (!canvasDrag.moved) {
      canvasDrag.moved = crossedCanvasDragThreshold(canvasDrag, event.clientX, event.clientY, 3);
    }
    if (!canvasDrag.moved) return;
    const rect = playerCanvas.getBoundingClientRect();
    const position = canvasDragPosition(canvasDrag, event.clientX, event.clientY, rect.width, rect.height);
    if (!position) return;
    canvasDrag.lastX = position.x;
    canvasDrag.lastY = position.y;
  });
  playerCanvas.addEventListener("pointerup", (event) => {
    const drag = canvasDrag;
    canvasDrag = null;
    if (!drag || event.pointerId !== drag.pointerId || !drag.moved) return;
    void queueWorkingCopy(() => editTimelineClip(workingCopy, drag.timelinePath, (target) => {
      if (target.band !== "visual") throw new Error("selected item is not a visual clip");
      target.clip.position = [drag.lastX, drag.lastY];
    }));
  });
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape") canvasDrag = null;
  });
  window.addEventListener("resize", positionSelectionBox);

  inspector.addEventListener("studio-inspector-intent", (event) => {
    const intent = (event as CustomEvent<StudioInspectorIntent>).detail;
    if (intent.type === "edit") {
      void editItem(intent.clipId, intent.key, intent.value).then(renderInspector);
    } else if (intent.type === "delete") {
      void deleteItem(intent.clipId);
    } else {
      void openMotionClip(intent.clipId).catch((error) => {
        rendererMessage = error instanceof Error ? error.message : String(error);
        renderMeta();
      });
    }
  });

  let externalUpdatePending = false;

  function consumeExternalUpdate(): void {
    if (!externalUpdatePending) return;
    externalUpdatePending = false;
    if (dirty || conflictMessage !== null) {
      if (conflictMessage === null) {
        setConflict(`external project update after revision ${baseRevision}`);
      }
    } else {
      void reloadProject();
    }
  }

  async function saveOneSnapshot(): Promise<boolean> {
    if (!saveTimeline) throw new Error("Studio project save capability is unavailable");
    const localSnapshotJson = JSON.stringify(workingCopy);
    const submitted = captureTimelineSaveSnapshot(workingCopy);
    const response = await saveTimeline({
      baseRevision,
      timeline: submitted.timeline,
      intent: "studio save",
    });
    if (response.outcome === "staleBase") {
      setConflict(
        `stale base · HEAD is revision ${response.revision}; reload explicitly before saving again`,
      );
      return false;
    }
    if (response.outcome === "rejected") {
      setConflict(response.errors.map((error) => error.code).join(", ") || "edit rejected");
      return false;
    }
    const next = await loadTimeline() as StudioConfig;
    let committed;
    try {
      committed = assertReloadedTimelineSave(
        response,
        next,
        submitted,
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setConflict(`saved revision reload failed: ${message}`);
      return false;
    }
    config = next;
    motionProjection = isMotionProjection(next.motion) ? structuredClone(next.motion) : null;
    baseRevision = next.timelineRevision.revision;
    const unchanged = JSON.stringify(workingCopy) === localSnapshotJson;
    try {
      if (await replacePreview(next)) rendererMessage = null;
    } catch (error) {
      rendererMessage = error instanceof Error ? error.message : String(error);
    }
    if (unchanged) {
      workingCopy = committed;
      compiledCopy = compileTimeline(committed);
      documentView = compiledCopy.view;
      fps = documentView.canvas.framesPerSecond;
      dirty = false;
      shell.dispatchIntent({ type: "dirty", value: false });
    } else {
      saveQueue.noteEdit();
    }
    setConflict(null);
    renderWorkingCopy();
    renderMeta();
    return true;
  }

  const saveQueue = new TimelineSaveQueue(saveOneSnapshot, consumeExternalUpdate);

  function requestSave(): Promise<boolean> {
    if (conflictMessage !== null) setConflict(null);
    return saveQueue.request();
  }

  async function reloadProject(): Promise<void> {
    const next = await loadTimeline() as StudioConfig;
    config = next;
    workingCopy = structuredClone(next.timeline);
    compiledCopy = compileTimeline(workingCopy);
    documentView = compiledCopy.view;
    fps = documentView.canvas.framesPerSecond;
    player.style.aspectRatio = `${documentView.canvas.width} / ${documentView.canvas.height}`;
    motionProjection = isMotionProjection(next.motion) ? structuredClone(next.motion) : null;
    baseRevision = next.timelineRevision.revision;
    dirty = false;
    shell.dispatchIntent({ type: "dirty", value: false });
    setConflict(null);
    renderWorkingCopy();
    try {
      if (await replacePreview(next)) rendererMessage = null;
    } catch (error) {
      rendererMessage = error instanceof Error ? error.message : String(error);
    }
    renderMeta();
  }

  projectControls.addEventListener("studio-project-intent", (event) => {
    const intent = (event as CustomEvent<StudioProjectIntent>).detail;
    if (intent.type === "save") void requestSave();
    else void reloadProject();
  });

  shell.addEventListener("studio-host-event", (event) => {
    const detail = (event as CustomEvent<{ type: string; data: unknown }>).detail;
    if (detail.type !== "timeline" && detail.type !== "project") return;
    if (saveQueue.active) {
      externalUpdatePending = true;
      return;
    }
    if (dirty) {
      setConflict(`external project update after revision ${baseRevision}`);
    } else {
      void reloadProject();
    }
  });

  function currentMotionContext(clipId: string): GoodMotionContext {
    if (boot.session.kind !== "project" || !motionProjection) {
      throw new Error("Project Motion context is unavailable");
    }
    const context = projectMotionContextFromAdmittedPreview(
      { ...config, motion: motionProjection },
      boot as typeof boot & { session: Extract<typeof boot.session, { kind: "project" }> },
      { clipId },
      compiledCopy.timeline,
    );
    if (context.status !== "ok") throw new Error(context.diagnostics.map((item) => item.message).join("\n"));
    const motionFrames = findProjectedItem(compiledCopy.timeline, documentView, clipId)?.motionFrames;
    return motionContextWithTimelineFrames(context, motionFrames);
  }

  async function openMotionClip(clipId: string): Promise<void> {
    if (!options.openMotion || boot.session.kind !== "project" || !motionProjection) {
      throw new Error("Motion editing requires a project Studio session");
    }
    if (!previewAvailable) {
      throw new Error("Motion editing requires a successfully admitted fixed-package preview");
    }
    const projected = findProjectedItem(compiledCopy.timeline, documentView, clipId);
    const clip = projected && visualClip(projected.item);
    if (!projected?.timelinePath || !clip || clip.source.type !== "motion") {
      throw new Error(`'${clipId}' is not a Motion visual clip`);
    }
    const timelinePath = projected.timelinePath;
    const context = currentMotionContext(clipId);
    const session: ProjectMotionSession = {
      clipId,
      context,
      props: preparedMotionProps(compiledCopy.timeline, motionProjection, clipId, context.artifact),
      clipStartS: projected.startSeconds,
      applyEdit: async (edit: ProjectMotionEdit) => {
        const next = edit.type === "phase"
          ? editTimelineMotionFrames(workingCopy, timelinePath, {
            type: "motionPhaseFrames",
            phase: edit.key === "enterFrames" ? "enter" : edit.key === "exitFrames" ? "exit" : (() => {
              throw new Error(`unknown Motion phase field '${edit.key}'`);
            })(),
            frames: edit.value,
          }, timelineTimeFromFrames)
          : edit.type === "cue"
            ? editTimelineMotionFrames(workingCopy, timelinePath, {
              type: "motionCueFrames",
              cue: edit.cue,
              field: edit.key === "startFrame" ? "start"
                : edit.key === "endFrame" ? "end"
                : edit.key === "enterFrames" ? "enter"
                : edit.key === "exitFrames" ? "exit"
                : (() => { throw new Error(`unknown Motion cue field '${edit.key}'`); })(),
              frames: edit.value,
            }, timelineTimeFromFrames)
            : setTimelineMotionProp(
              workingCopy,
              timelinePath,
              edit.name,
              edit.value.value,
            );
        await commitWorkingCopy(next);
        const context = currentMotionContext(clipId);
        return {
          context,
          props: preparedMotionProps(
            compiledCopy.timeline,
            motionProjection,
            clipId,
            context.artifact,
          ),
        };
      },
      returnToTimeline: async () => {
        renderWorkingCopy();
        if (previewAvailable) {
          await player.seek(Math.min(player.currentTime(), player.lastFrameTimeS()));
        }
      },
    };
    await options.openMotion(session);
  }

  async function runSmoke(): Promise<void> {
    if (!previewAvailable) throw new Error("Studio smoke capture requires an admitted preview");
    const captures: Array<{ sampleId: string; timeS: number; pngBase64: string }> = [];
    for (const capture of config.captures ?? []) {
      const rendered = await player.scrub(capture.timeS);
      if (!rendered.png) throw new Error("Studio smoke capture did not return PNG bytes");
      captures.push({ ...capture, pngBase64: bytesToBase64(rendered.png) });
    }
    await fetch("/result", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        status: "ok",
        sequenceTracks: projectTimelineSequences(compiledCopy.timeline, documentView).length,
        rendererMessage,
        captures,
      }),
    });
  }

  async function runProjectAuthoringSmoke(): Promise<void> {
    if (boot.session.kind !== "project") {
      throw new Error("Project authoring smoke requires a project Studio session");
    }
    const beforeRevision = baseRevision;
    const marker = "#14532dff";
    const edited = structuredClone(workingCopy);
    edited.canvas.background = marker;
    await commitWorkingCopy(edited);
    if (!await requestSave()) throw new Error("Project authoring smoke save was not committed");
    if (baseRevision === beforeRevision) {
      throw new Error("Project authoring smoke did not advance the Project revision");
    }

    const reloaded = await loadTimeline() as StudioConfig;
    if (reloaded.timeline.canvas.background !== marker) {
      throw new Error("Project authoring smoke reload lost the complete working-copy edit");
    }
    if (reloaded.timelineRevision.revision !== baseRevision) {
      throw new Error("Project authoring smoke reload returned a different Timeline revision");
    }

    await fetch("/result", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        status: "ok",
        implementation: "valle-project-studio-authoring",
        userAgent: navigator.userAgent,
        stats: {
          previewAvailable,
          beforeRevision,
          afterRevision: baseRevision,
          marker,
        },
        captures: [],
      }),
    });
  }

  pixelsPerSecond = Math.max(
    2,
    Math.min(600, (timelineView.clientWidth - 142) / Math.max(0.001, documentView.canvas.durationSeconds)),
  );
  renderWorkingCopy();
  setSelected(selectedItemId);
  requiredElement("loading").hidden = true;
  await seek(config.initialTimeS ?? 0);
  syncTransport();
  const search = new URLSearchParams(location.search);
  const authoringSmokeToken = search.get("authoringSmoke");
  if (authoringSmokeToken !== null) {
    if (boot.session.kind !== "project" || authoringSmokeToken !== boot.session.token) {
      throw new Error("Project authoring smoke requires the bound Studio session token");
    }
    await runProjectAuthoringSmoke();
  } else if (search.get("smoke") === "1") await runSmoke();
}

window.addEventListener("error", (event) => {
  // Element/resource load failures also dispatch `error` at Window, but carry neither an
  // Error object nor a diagnostic message. They are handled by the owning loader and must not
  // race the Studio smoke/result channel as a fabricated "unknown error" runtime failure.
  const eventMessage = typeof event.message === "string" ? event.message.trim() : "";
  const failure = event.error ?? (eventMessage.length > 0 ? eventMessage : null);
  if (failure == null) return;
  const message = errorText(failure);
  showFatalError(message);
  void postFailure(message);
});
window.addEventListener("unhandledrejection", (event) => {
  const message = errorText(event.reason);
  showFatalError(message);
  void postFailure(message);
});

export async function startTimelineStudio(options: TimelineWorkspaceOptions = {}): Promise<void> {
  await main(options).catch(async (error) => {
    const message = errorText(error);
    showFatalError(message);
    await postFailure(message).catch(() => undefined);
  });
}
