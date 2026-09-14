import { DraftPreview } from "./draft-preview.ts";
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
  StudioInspector,
  StudioInspectorIntent,
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
    if (item.source.type === "motion") return (item.source.component.split(/[\\/]/).pop() ?? item.source.component).replace(/^resource:/, "");
    if (item.source.type === "solid") return item.source.color;
    return (item.source.resource.split(/[\\/]/).pop() ?? item.source.resource).replace(/^resource:/, "");
  }
  if ("type" in item && item.type === "clip" && "source" in item) return item.source.resource;
  if ("type" in item && item.type === "clip" && "runs" in item) {
    return item.runs.map((run: { text: string }) => run.text).join("");
  }
  if ("effect" in item) return item.effect.type;
  if ("type" in item) return item.type === "gap" ? "Gap" : item.type === "crossfade" ? "Audio crossfade" : "Transition";
  return "Adjustment";
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
  const transport = requiredElement<StudioTransport>("studioTransport");
  const timelineView = requiredElement<StudioTimelineElement>("tracks");
  const inspector = requiredElement<StudioInspector>("inspectorBody");
  const projectControls = requiredElement<StudioProjectControls>("projectControls");
  const player = requiredElement<VallePlayerElement>("studioPlayer");
  const inspectorBody = requiredElement<HTMLElement>("inspectorBody");
  const motionWorkspaceEl = requiredElement<HTMLElement>("motionWorkspace");
  const timelinePane = requiredElement<HTMLElement>("timelinePane");
  const zoomInput = requiredElement<HTMLInputElement>("timelineZoom");
  await Promise.all([
    transport.updateComplete,
    timelineView.updateComplete,
    inspector.updateComplete,
    projectControls.updateComplete,
    player.updateComplete,
  ]);
  inspectorBody.hidden = false;
  motionWorkspaceEl.hidden = true;
  timelinePane.hidden = false;
  shell.dispatchIntent({ type: "panel", inspectorVisible: true });
  shell.restorePanelPrefs();

  let config = await loadTimeline() as StudioConfig;
  let workingCopy: Timeline = structuredClone(config.timeline);
  let motionProjection: MotionProjection | null = isMotionProjection(config.motion)
    ? structuredClone(config.motion)
    : null;
  let baseRevision = config.timelineRevision.revision;
  let selectedItemId: string | null = shell.shellState.selectedClipId;
  let dirty = shell.shellState.dirty;
  let savedSnapshotJson = JSON.stringify(workingCopy);
  let saving = false;
  let conflictMessage: string | null = null;
  let pixelsPerSecond = 40;
  let playhead: HTMLElement | null = null;
  let canvasDrag: CanvasDragState | null = null;
  let sequenceDragPath: string | null = null;
  let timelineTrim: TimelineTrimState | null = null;
  const editQueue = new TimelineEditQueue();
  const canSaveTimeline = boot.capabilities.saveTimeline;
  const storageKind = boot.session.kind === "timeline-file" ? "file" : "project";
  const canvas = workingCopy.canvas;
  player.style.aspectRatio = `${canvas.width} / ${canvas.height}`;

  const historyStack: string[] = [];
  const redoStack: string[] = [];
  let restoringHistory = false;
  let historyTransaction: string | null = null;

  function snapshotJson(): string {
    return JSON.stringify(workingCopy);
  }

  function beginHistory(): void {
    if (restoringHistory || historyTransaction !== null) return;
    historyTransaction = snapshotJson();
  }

  function commitHistory(): void {
    if (restoringHistory || historyTransaction === null) return;
    const before = historyTransaction;
    historyTransaction = null;
    const after = snapshotJson();
    if (before === after) return;
    historyStack.push(before);
    if (historyStack.length > 40) historyStack.shift();
    redoStack.length = 0;
    updateHistoryChrome();
  }

  function updateHistoryChrome(): void {
    shell.dispatchIntent({
      type: "history",
      canUndo: historyStack.length > 0,
      canRedo: redoStack.length > 0,
    });
  }

  function restoreHistory(nextJson: string): void {
    restoringHistory = true;
    try {
      const next = JSON.parse(nextJson) as Timeline;
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
      scheduleDraftPreview();
      shell.dispatchEvent(new CustomEvent("studio-draft-updated"));
    } finally {
      restoringHistory = false;
      updateHistoryChrome();
    }
  }

  function undoHistory(): void {
    if (!historyStack.length) return;
    const current = snapshotJson();
    const previous = historyStack.pop()!;
    redoStack.push(current);
    restoreHistory(previous);
  }

  function redoHistory(): void {
    if (!redoStack.length) return;
    const current = snapshotJson();
    const next = redoStack.pop()!;
    historyStack.push(current);
    restoreHistory(next);
  }

  shell.addEventListener("studio-history-intent", (event) => {
    const intent = (event as CustomEvent<{ type: "undo" | "redo" }>).detail;
    if (intent.type === "undo") undoHistory();
    else redoHistory();
  });

  shell.addEventListener("studio-return-timeline", () => {
    // Timeline is already the active workspace in this module.
  });

  document.getElementById("fitTimeline")?.addEventListener("click", () => {
    if (shell.workspace.kind !== "timeline") return;
    setTimelineZoom(1);
    requiredElement<HTMLElement>("timelineScroll").scrollLeft = 0;
  });
  document.getElementById("zoomIn")?.addEventListener("click", () => {
    if (shell.workspace.kind !== "timeline") return;
    setTimelineZoom(zoomScale + 0.25);
  });
  document.getElementById("zoomOut")?.addEventListener("click", () => {
    if (shell.workspace.kind !== "timeline") return;
    setTimelineZoom(zoomScale - 0.25);
  });
  zoomInput.addEventListener("input", () => {
    if (shell.workspace.kind !== "timeline") return;
    setTimelineZoom(Number(zoomInput.value));
  });

  let zoomScale = 1;

  function setTimelineZoom(next: number): void {
    const clamped = Math.min(8, Math.max(0.25, next));
    zoomScale = clamped;
    zoomInput.value = String(clamped);
    const scroll = requiredElement<HTMLElement>("timelineScroll");
    const offset = currentTime() * pixelsPerSecond - scroll.scrollLeft;
    fitPixelsPerSecond(clamped);
    renderTimeline();
    scroll.scrollLeft = Math.max(0, currentTime() * pixelsPerSecond - offset);
  }

  function fitPixelsPerSecond(multiplier: number): void {
    const duration = Math.max(0.001, documentView.canvas.durationSeconds);
    const available = Math.max(200, requiredElement<HTMLElement>("timelineScroll").clientWidth - 152);
    pixelsPerSecond = Math.max(2, Math.min(600, (available / duration) * multiplier));
  }

  function kindClassOf(kind: string): string {
    const value = kind.toLowerCase();
    if (value.includes("motion") || value.includes("lottie")) return "kind-motion";
    if (value.includes("audio")) return "kind-audio";
    if (value.includes("caption") || value.includes("text")) return "kind-caption";
    if (value.includes("effect") || value.includes("adjustment")) return "kind-effect";
    return "kind-visual";
  }

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
  let editError: string | null = null;
  if (!previewAvailable) player.loadingLabel = "Preparing preview…";

  const preparePreview = studioHost.prepareTimelinePreview?.bind(studioHost);
  const draftPreview = new DraftPreview<Timeline, Awaited<ReturnType<NonNullable<StudioHost["prepareTimelinePreview"]>>>>({
    prepare: (draft) => {
      if (!preparePreview) throw new Error("Draft preview is unavailable for this session");
      return preparePreview({ timeline: draft });
    },
    apply: async (result, draft, isCurrent) => {
      if (result.status === "error") throw new Error(result.diagnostics.map((item) => item.message).join("\n"));
      const next: StudioConfig = { ...config, timeline: draft, timelineJson: JSON.stringify(draft),
        render: result.render, assets: result.assets as StudioConfig["assets"], motion: result.motion };
      if (!await replacePreview(next, isCurrent) || !isCurrent()) return;
      config = next;
      motionProjection = isMotionProjection(result.motion) ? structuredClone(result.motion) : null;
      syncTransport();
      if (shell.workspace.kind === "timeline") renderTimeline();
      shell.dispatchEvent(new CustomEvent("studio-draft-updated"));
    },
    updating: () => shell.dispatchIntent({ type: "preview-status", status: "updating", message: null }),
    failed: (error) => {
      rendererMessage = error instanceof Error ? error.message : String(error);
      shell.dispatchIntent({ type: "preview-status", status: "error", message: rendererMessage });
    },
    ready: () => {
      rendererMessage = null;
      requiredElement("errbox").hidden = true;
      shell.dispatchIntent({ type: "preview-status", status: "ready", message: null });
      renderMeta();
    },
  });
  function scheduleDraftPreview(): void { if (preparePreview) draftPreview.schedule(workingCopy); }
  async function flushDraftPreview(): Promise<void> { await draftPreview.flush(); }
  shell.addEventListener("studio-preview-retry", () => { scheduleDraftPreview(); void flushDraftPreview(); });
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

  async function replacePreview(next: StudioConfig, isCurrent: () => boolean = () => true): Promise<boolean> {
    const replacement = fixedPackageReplacement(next);
    if (!replacement) return false;
    if (playerLoaded) {
      await player.replaceRenderPackage({ ...replacement, isCurrent });
    } else {
      await player.load({ ...replacement, runtimeAssets: next.runtimeAssets, runtimeBaseUrl: next.runtimeBaseUrl ?? location.href });
      playerLoaded = true;
      if (isCurrent()) await player.seek(editingTimeS);
    }
    if (!isCurrent()) return false;
    previewAvailable = true;
    transport.disabled = false;
    player.setMuted(transport.muted);
    globalThis.valleStudioPlayer = player;
    return true;
  }

  function stageMetaText(): string {
    const currentCanvas = workingCopy.canvas;
    const duration = documentView.canvas.durationSeconds;
    return `${currentCanvas.width} × ${currentCanvas.height} · ${fps.toFixed(Number.isInteger(fps) ? 0 : 3)} fps · ${formatSeconds(duration)}`;
  }

  function countTracks(): number {
    return projectTimelineSequences(compiledCopy.timeline, documentView).length;
  }

  function renderMeta(): void {
    if (shell.workspace.kind === "timeline") {
      shell.style.setProperty("--canvas-width", `${workingCopy.canvas.width}px`);
      shell.style.setProperty("--canvas-height", `${workingCopy.canvas.height}px`);
      shell.style.setProperty("--canvas-aspect", String(workingCopy.canvas.width / workingCopy.canvas.height));
    }
    document.getElementById("stageMeta")!.textContent = stageMetaText();
    document.getElementById("trackCount")!.textContent = `${countTracks()} tracks`;
    document.getElementById("timelineSelection")!.textContent = selectedItemId
      ? `Selected · ${selectedItemId}`
      : "";
    document.getElementById("statusbarRight")!.textContent = canSaveTimeline
      ? storageKind === "file" ? "Save writes the opened JSON file" : `Revision ${baseRevision}`
      : "Read-only preview session";
    projectControls.sync({ visible: canSaveTimeline, dirty, saving, conflictMessage });
    shell.dispatchIntent({ type: "dirty", value: dirty });
    updateHistoryChrome();
  }

  function markDirty(): void {
    dirty = JSON.stringify(workingCopy) !== savedSnapshotJson;
    shell.dispatchIntent({ type: "dirty", value: dirty });
    renderMeta();
  }

  function setConflict(message: string | null): void {
    conflictMessage = message;
    shell.dispatchIntent({
      type: "conflict",
      value: message !== null,
      message,
    });
    renderMeta();
  }

  function sequenceView(): TimelineViewModel {
    const duration = Math.max(0.001, documentView.canvas.durationSeconds);
    const labelWidth = 152;
    const laneWidth = Math.max(1, Math.ceil(duration * pixelsPerSecond));
    const tickSeconds = pixelsPerSecond >= 120 ? 1 : pixelsPerSecond >= 30 ? 5 : 10;
    const ticks: TimelineViewModel["ticks"][number][] = [];
    for (let second = 0; second <= duration; second += tickSeconds) {
      ticks.push({
        leftPx: second * pixelsPerSecond,
        label: Number.isInteger(second) ? `${Math.floor(second / 60)}:${String(second % 60).padStart(2, "0")}` : `${Math.floor(second)}s`,
      });
      if (tickSeconds > 1) {
        const half = second + tickSeconds / 2;
        if (half < duration) ticks.push({ leftPx: half * pixelsPerSecond, label: null });
      }
    }
    const tracks = projectTimelineSequences(compiledCopy.timeline, documentView).map((track) => ({
      id: track.id,
      kind: track.band,
      name: `${track.band.charAt(0).toUpperCase()}${track.band.slice(1)} ${track.index + 1}`,
      indexLabel: track.band.slice(0, 1).toUpperCase(),
      selected: Boolean(selectedItemId && track.items.some((entry) => entry.item.id === selectedItemId)),
      clips: track.items.filter((entry) => !("type" in entry.item && entry.item.type === "gap")).map((entry) => {
        const durationBearing = entry.advancesCursor || entry.band === "adjustment";
        const kind = sourceKind(entry.item);
        return {
          id: entry.item.id,
          kind,
          leftPx: Math.max(0, entry.startSeconds * pixelsPerSecond),
          widthPx: durationBearing
            ? Math.max(0, entry.durationSeconds * pixelsPerSecond)
            : 3,
          title: durationBearing
            ? `${entry.item.id} · ${formatSeconds(entry.startSeconds)} · ${entry.item.duration}`
            : `${entry.item.id} · cut ${formatSeconds(entry.startSeconds)} · nominal ${entry.item.duration}`,
          label: itemLabel(entry.item),
          selected: selectedItemId === entry.item.id,
          timingEditable: durationBearing && entry.timelinePath !== null,
          readOnly: entry.timelinePath === null,
          media: mediaFor(entry),
        };
      }),
    }));
    return {
      widthPx: labelWidth + laneWidth,
      laneWidthPx: laneWidth,
      labelWidthPx: labelWidth,
      playheadLeftPx: labelWidth + currentTime() * pixelsPerSecond,
      ticks,
      tracks,
      showPlayhead: true,
    };
  }

  type MediaView = NonNullable<TimelineViewModel["tracks"][number]["clips"][number]["media"]>;
  type Samples = { kind: "video"; frames: Awaited<ReturnType<Player["videoKeyframeThumbnails"]>> }
    | { kind: "audio"; peaks: Float32Array; durationS: number };
  const mediaSamples = new Map<string, Promise<Samples>>();
  function mediaFor(entry: ProjectedSequenceItem): MediaView | undefined {
    const kind = entry.band === "audio" && "type" in entry.item && entry.item.type === "clip" ? "audio" : sourceKind(entry.item) === "video" ? "video" : null;
    if (!kind || !previewAvailable) return undefined;
    const tracks = kind === "audio" ? config.render.timeline.document.audio.tracks : config.render.timeline.document.visual.tracks;
    const admitted = tracks.flatMap<AudioItem | VisualItem>((track) => track.items).find((item) => item.id === entry.item.id);
    if (!admitted || admitted.type !== "clip" || !("resource" in admitted.source)) return undefined;
    return { kind, assetId: admitted.source.resource, sourceStartS: entry.sourceStartSeconds ?? 0,
      sourceDurationS: entry.durationSeconds * (entry.sourceRate ?? 1) };
  }
  async function paintTimelineMedia(model: TimelineViewModel): Promise<void> {
    await timelineView.updateComplete;
    for (const clip of model.tracks.flatMap((track) => track.clips)) {
      const media = clip.media;
      if (!media) continue;
      const block = Array.from(timelineView.querySelectorAll<HTMLElement>("[data-clip-id]")).find((node) => node.dataset.clipId === clip.id);
      const canvas = block?.querySelector("canvas");
      if (!canvas) continue;
      const url = config.assets?.find((asset) => asset.id === media.assetId)?.url ?? media.assetId;
      const key = `${media.kind}:${url}`;
      let samples = mediaSamples.get(key);
      if (!samples) {
        samples = media.kind === "video"
          ? player.videoKeyframeThumbnails(media.assetId, { height: 30, maxCount: 48 }).then((frames) => ({ kind: "video" as const, frames }))
          : player.audioPeaks(media.assetId).then((result) => ({ kind: "audio" as const, ...result }));
        mediaSamples.set(key, samples);
      }
      void samples.then((data) => {
        if (!canvas.isConnected || shell.workspace.kind !== "timeline") return;
        const width = Math.max(1, Math.ceil(clip.widthPx)), height = 28;
        canvas.width = width; canvas.height = height;
        const drawing = canvas.getContext("2d");
        if (!drawing) return;
        if (data.kind === "video") {
          if (!data.frames.length) throw new Error("No video samples");
          const tile = Math.max(1, height * data.frames[0]!.bitmap.width / data.frames[0]!.bitmap.height);
          for (let x = 0; x < width; x += tile) {
            const time = media.sourceStartS + x / width * media.sourceDurationS;
            const sample = data.frames.reduce((best, frame) => Math.abs(frame.tS - time) < Math.abs(best.tS - time) ? frame : best);
            drawing.drawImage(sample.bitmap, x, 0, tile, height);
          }
          canvas.title = "Source keyframes";
        } else {
          drawing.strokeStyle = "#78b7a0"; drawing.lineWidth = 1; drawing.beginPath();
          for (let x = 0; x < width; x += 2) {
            const time = media.sourceStartS + x / width * media.sourceDurationS;
            const index = Math.floor(time / Math.max(1e-9, data.durationS) * data.peaks.length);
            const peak = index >= 0 && index < data.peaks.length ? data.peaks[index]! : 0;
            drawing.moveTo(x, height / 2 - peak * (height / 2 - 1));
            drawing.lineTo(x, height / 2 + Math.max(0.5, peak * (height / 2 - 1)));
          }
          drawing.stroke(); canvas.title = "Source audio waveform";
        }
      }).catch(() => {
        if (!canvas.isConnected) return;
        canvas.replaceWith(Object.assign(document.createElement("span"), { className: "media-unavailable", textContent: "Preview unavailable" }));
      });
    }
  }
  window.addEventListener("beforeunload", () => { for (const samples of mediaSamples.values()) void samples.then((data) => { if (data.kind === "video") data.frames.forEach((frame) => frame.bitmap.close()); }).catch(() => {}); });

  function renderTimeline(): void {
    const model = sequenceView();
    timelineView.renderTimeline(model);
    void paintTimelineMedia(model);
    playhead = timelineView.querySelector<HTMLElement>("#tlPlayhead");
    positionPlayhead();
  }

  function positionPlayhead(): void {
    if (playhead) playhead.style.left = `${152 + currentTime() * pixelsPerSecond}px`;
    const grip = timelineView.querySelector<HTMLElement>("#rulerGrip");
    if (grip) grip.style.left = `${currentTime() * pixelsPerSecond - 5}px`;
  }

  function selectedProjection(): ProjectedSequenceItem | null {
    return selectedItemId
      ? findProjectedItem(compiledCopy.timeline, documentView, selectedItemId)
      : null;
  }

  function renderInspector(): void {
    const canvas = workingCopy.canvas;
    const duration = documentView.canvas.durationSeconds;
    const totalFrames = Math.round(duration * fps);
    const projectSummary = [
      { label: "Canvas", value: `${canvas.width} × ${canvas.height}` },
      { label: "Frame rate", value: `${fps} fps` },
      { label: "Duration", value: `${formatSeconds(duration)} · ${totalFrames} f` },
      ...(storageKind === "file" ? [{ label: "Save", value: "Writes the opened JSON file" }] : [{ label: "Revision", value: String(baseRevision) }]),
    ];
    const projected = selectedProjection();
    if (!projected) {
      inspector.renderInspector({
      errorMessage: editError,
        clipId: null,
        kindLabel: storageKind === "file" ? "Timeline file" : "Project",
        title: shell.shellState.projectName,
        summary: projectSummary,
        sections: [],
        emptyHint: "Select a clip on the stage or timeline to inspect it.",
      });
      return;
    }
    const item = projected.item;
    const kind = sourceKind(item);
    const kindClass = kindClassOf(kind);
    const sections: InspectorSectionView[] = [{
      title: "Timing",
      rows: [
        { kind: "value", label: "Start", value: formatSeconds(projected.startSeconds) },
        ...(projected.timelinePath
          ? [{
              kind: "number" as const,
              key: "timing:durationFrames",
              label: "Duration",
              value: projected.durationFrames,
              min: 1,
              step: 1,
              unit: "f",
            }]
          : [{ kind: "value" as const, label: "Duration", value: `${projected.durationFrames} f` }]),
        { kind: "value", label: "Exact duration", value: String(item.duration) },
        ...(projected.timelinePath ? [] : [{
          kind: "value" as const,
          label: "Editability",
          value: "Compiler-generated · read only",
        }]),
      ],
    }];
    let motionSource: string | undefined;
    const visual = visualClip(item);
    if (visual) {
      const rows: InspectorSectionView["rows"][number][] = [
        { kind: "value", label: "Source", value: visual.source.type },
      ];
      if (visual.source.type === "solid") {
        rows.push({ kind: "color", key: "source:color", label: "Color", value: visual.source.color });
      }
      if (visual.source.type === "video" || visual.source.type === "lottie" || visual.source.type === "motion") {
        rows.push(
          { kind: "number", key: "timing:sourceStartFrames", label: "Source start", value: projected.sourceStartFrame ?? 0, min: 0, step: 1, unit: "f" },
          { kind: "value", label: "Source start (exact)", value: visual.source.sourceStart },
        );
      }
      if (visual.layer.opacity.type === "constant") {
        rows.push({
          kind: "number",
          key: "layer:opacity",
          label: "Opacity",
          value: Math.round(visual.layer.opacity.value * 100),
          min: 0,
          max: 100,
          step: 1,
          unit: "%",
        });
      } else {
        rows.push({ kind: "value", label: "Opacity", value: "Curve-driven · read only" });
      }
      if (visual.source.type === "video") {
        const gain = visual.source.gain;
        if (!gain || gain.type === "constant") {
          rows.push({ kind: "number", key: "source:gain", label: "Volume", value: gain?.value ?? 1, min: 0, step: 0.1 });
        } else {
          rows.push({ kind: "value", label: "Volume", value: "Curve-driven · read only" });
        }
      }
      const { position, scale, rotation, size } = visual.layer.transform;
      if (size?.type === "constant") {
        rows.push(
          { kind: "number", key: "layer:size-x", label: "Width", value: size.value[0], min: 1, step: 1, unit: "px" },
          { kind: "number", key: "layer:size-y", label: "Height", value: size.value[1], min: 1, step: 1, unit: "px" },
        );
      } else if (size) {
        rows.push({ kind: "value", label: "Size", value: "Curve-driven · read only" });
      }
      if (position.type === "constant") {
        rows.push(
          { kind: "number", key: "layer:position-x", label: "Position X", value: position.value[0] * workingCopy.canvas.width, step: 1, unit: "px", prefix: "X" },
          { kind: "number", key: "layer:position-y", label: "Position Y", value: position.value[1] * workingCopy.canvas.height, step: 1, unit: "px", prefix: "Y" },
        );
      } else {
        rows.push({ kind: "value", label: "Position", value: "Curve-driven · read only" });
      }
      if (scale.type === "constant") {
        rows.push(
          { kind: "number", key: "layer:scale-x", label: "Scale X", value: scale.value[0], step: 0.01 },
          { kind: "number", key: "layer:scale-y", label: "Scale Y", value: scale.value[1], step: 0.01 },
        );
      } else {
        rows.push({ kind: "value", label: "Scale", value: "Curve-driven · read only" });
      }
      if (rotation.type === "constant") {
        rows.push({
          kind: "number",
          key: "layer:rotation",
          label: "Rotation",
          value: rotation.value * 180 / Math.PI,
          step: 1,
          unit: "°",
        });
      } else {
        rows.push({ kind: "value", label: "Rotation", value: "Curve-driven · read only" });
      }
      if (visual.source.type === "motion") motionSource = visual.source.component;
      sections.push({ title: "Appearance", rows });
    }
    const audio = audioClip(item);
    if (audio) {
      const rows: InspectorSectionView["rows"][number][] = [
        { kind: "value", label: "Resource", value: audio.source.resource },
        { kind: "number", key: "timing:sourceStartFrames", label: "Source start", value: projected.sourceStartFrame ?? 0, min: 0, step: 1, unit: "f" },
        { kind: "value", label: "Source start (exact)", value: audio.source.sourceStart },
      ];
      if (audio.gain.type === "constant") {
        rows.push({ kind: "number", key: "audio:gain", label: "Gain", value: audio.gain.value, min: 0, step: 0.01 });
      } else {
        rows.push({ kind: "value", label: "Gain", value: "Curve-driven · read only" });
      }
      if (audio.pan.type === "constant") {
        rows.push({ kind: "number", key: "audio:pan", label: "Pan", value: audio.pan.value, min: -1, max: 1, step: 0.01 });
      } else {
        rows.push({ kind: "value", label: "Pan", value: "Curve-driven · read only" });
      }
      sections.push({ title: "Sound", rows });
    }
    const caption = captionItem(item);
    if (caption) {
      sections.push({
        title: "Content",
        rows: [
          ...caption.runs.map((run: { id: string; text: string }, runIndex: number) => ({
            kind: "textarea" as const,
            key: `caption:run:${runIndex}`,
            label: run.id,
            value: run.text,
            primaryText: true,
          })),
          { kind: "color", key: "caption:color", label: "Color", value: caption.style.color },
          { kind: "number", key: "caption:font-size", label: "Font size", value: caption.style.fontSize, min: 1, step: 1, unit: "px" },
        ],
      });
    }
    inspector.renderInspector({
      errorMessage: editError,
      clipId: item.id,
      kindLabel: kind,
      kindClass,
      title: itemLabel(item),
      subtitle: item.id,
      sections,
      rawJson: JSON.stringify(item, null, 2),
      motionSource,
      canDelete: Boolean(projected.timelinePath),
    });
  }

  function setSelected(itemId: string | null): void {
    editError = null;
    selectedItemId = itemId && findProjectedItem(compiledCopy.timeline, documentView, itemId)
      ? itemId
      : null;
    shell.dispatchIntent({ type: "select", clipId: selectedItemId, canOpenMotion: Boolean(selectedProjection() && sourceKind(selectedProjection()!.item) === "motion") });
    renderTimeline();
    renderInspector();
    positionSelectionBox();
  }

  function renderWorkingCopy(): void {
    if (shell.workspace.kind !== "timeline") { renderMeta(); return; }
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
    if (JSON.stringify(normalized) === JSON.stringify(workingCopy)) return;
    const compiled = compileTimeline(normalized);
    beginHistory();
    editError = null;
    workingCopy = structuredClone(normalized);
    compiledCopy = compiled;
    documentView = compiled.view;
    fps = documentView.canvas.framesPerSecond;
    player.style.aspectRatio = `${documentView.canvas.width} / ${documentView.canvas.height}`;
    selectedItemId = selectedItemId && findProjectedItem(compiledCopy.timeline, documentView, selectedItemId)
      ? selectedItemId
      : null;
    commitHistory();
    markDirty();
    renderWorkingCopy();
    shell.dispatchIntent({
      type: "preview-status",
      status: "updating",
      message: "Preparing draft preview…",
    });
    scheduleDraftPreview();
  }

  function queueWorkingCopy(next: () => Timeline): Promise<void> {
    return editQueue.enqueue(
      () => commitWorkingCopy(next()),
      (error) => {
        rendererMessage = error instanceof Error ? error.message : String(error);
        editError = rendererMessage;
        renderInspector();
        renderMeta();
      },
    );
  }

  function editItem(itemId: string, key: string, value: string | number | boolean): Promise<void> {
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
          // Inspector presents opacity as 0–100; Timeline stores 0–1.
          target.clip.opacity = Math.min(1, Math.max(0, Number(value) / 100));
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
          current[key.endsWith("-x") ? 0 : 1] = Number(value) / (field === "position" ? (key.endsWith("-x") ? workingCopy.canvas.width : workingCopy.canvas.height) : 1);
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
      else {
        animationFrame = null;
        if (shell.workspace.kind === "timeline" && transport.looping && player.currentTime() >= player.lastFrameTimeS() - 1e-6) {
          void player.seek(0).then(() => player.play()).then(startTransportPoll);
        }
      }
    };
    animationFrame = requestAnimationFrame(tick);
  }

  transport.addEventListener("studio-transport-intent", (event) => {
    if (shell.workspace.kind !== "timeline") return;
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
    } else if (intent.type === "play") {
      if (previewAvailable) void player.play().then(startTransportPoll);
    } else if (intent.type === "first-frame") {
      void seek(0).then(syncTransport);
    } else if (intent.type === "last-frame") {
      void seek(documentView.canvas.durationSeconds).then(syncTransport);
    } else if (intent.type === "seek-frame") {
      const timeS = timelineTimeFromFrames(intent.frame, workingCopy.canvas.fps);
      void seek(timeS).then(syncTransport);
    } else if (intent.type === "loop") {
      // Loop is a transport preference; player owns end-of-clip behavior.
    } else if (intent.type === "mute") {
      if (previewAvailable) player.setMuted(intent.value);
    } else {
      void seek(intent.timeS).then(syncTransport);
    }
  });
  player.addEventListener("time", () => { if (shell.workspace.kind === "timeline") syncTransport(); });

  timelineView.addEventListener("click", (event) => {
    if (shell.workspace.kind !== "timeline") return;
    const target = event.target instanceof Element ? event.target : null;
    const itemElement = target?.closest<HTMLElement>("[data-clip-id]");
    if (itemElement?.dataset.clipId) {
      setSelected(itemElement.dataset.clipId);
      return;
    }
    const ruler = target?.closest<HTMLElement>(".ruler-lane");
    if (ruler) {
      const rect = ruler.getBoundingClientRect();
      void seek(Math.max(0, (event.clientX - rect.left) / pixelsPerSecond)).then(syncTransport);
    }
  });

  timelineView.addEventListener("pointerdown", (event) => {
    if (shell.workspace.kind !== "timeline") return;
    const target = event.target instanceof Element ? event.target : null;
    const handle = target?.closest<HTMLElement>(".trim");
    if (handle) {
      event.preventDefault();
      const itemId = handle.closest<HTMLElement>("[data-clip-id]")?.dataset.clipId;
      if (!itemId) return;
      const projected = findProjectedItem(compiledCopy.timeline, documentView, itemId);
      if (!projected?.timelinePath) return;
      timelineTrim = {
        timelinePath: projected.timelinePath,
        edge: handle.classList.contains("left") ? "start" : "end",
        pointerId: event.pointerId,
        startX: event.clientX,
      };
      sequenceDragPath = null;
      showDragTip(event.clientX, event.clientY, "Trimming…");
      timelineView.setPointerCapture(event.pointerId);
      return;
    }
    const block = target?.closest<HTMLElement>("[data-clip-id]");
    const itemId = block?.dataset.clipId;
    const projected = itemId
      ? findProjectedItem(compiledCopy.timeline, documentView, itemId)
      : null;
    if (projected?.band === "visual" && projected.timelinePath && visualClip(projected.item)) {
      sequenceDragPath = projected.timelinePath;
    }
    const lane = target?.closest<HTMLElement>(".ruler-lane");
    if (lane) {
      event.preventDefault();
      const rect = lane.getBoundingClientRect();
      void seek(Math.max(0, (event.clientX - rect.left) / pixelsPerSecond)).then(syncTransport);
      const move = (moveEvent: PointerEvent) => {
        if (moveEvent.pointerId !== event.pointerId) return;
        void seek(Math.max(0, (moveEvent.clientX - rect.left) / pixelsPerSecond)).then(syncTransport);
      };
      const up = (upEvent: PointerEvent) => {
        if (upEvent.pointerId !== event.pointerId) return;
        lane.removeEventListener("pointermove", move);
        lane.removeEventListener("pointerup", up);
        lane.removeEventListener("pointercancel", up);
      };
      lane.addEventListener("pointermove", move);
      lane.addEventListener("pointerup", up);
      lane.addEventListener("pointercancel", up);
    }
  });
  timelineView.addEventListener("pointermove", (event) => {
    if (shell.workspace.kind !== "timeline") return;
    if (!timelineTrim) return;
    const pixelsPerFrame = pixelsPerSecond / documentView.canvas.framesPerSecond;
    const deltaFrames = Math.round((event.clientX - timelineTrim.startX) / pixelsPerFrame);
    showDragTip(event.clientX, event.clientY, `${deltaFrames >= 0 ? "+" : ""}${deltaFrames} f`);
  });
  timelineView.addEventListener("pointercancel", () => {
    timelineTrim = null; sequenceDragPath = null; hideDragTip();
  });
  timelineView.addEventListener("pointerup", (event) => {
    if (shell.workspace.kind !== "timeline") return;
    hideDragTip();
    const trim = timelineTrim;
    timelineTrim = null;
    if (trim && event.pointerId === trim.pointerId) {
      if (timelineView.hasPointerCapture(event.pointerId)) {
        timelineView.releasePointerCapture(event.pointerId);
      }
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
      ? event.target.closest<HTMLElement>("[data-clip-id]")?.dataset.clipId
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
    hideDragTip();
    if (timelineTrim?.pointerId === event.pointerId) timelineTrim = null;
  });

  function showDragTip(x: number, y: number, text: string): void {
    const tip = requiredElement<HTMLElement>("dragTip");
    tip.hidden = false;
    tip.textContent = text;
    tip.style.left = `${Math.min(innerWidth - 210, Math.max(8, x + 12))}px`;
    tip.style.top = `${y - 38}px`;
  }

  function hideDragTip(): void {
    requiredElement<HTMLElement>("dragTip").hidden = true;
  }

  const selectionBox = requiredElement<HTMLElement>("selBox");
  const stageWrap = requiredElement<HTMLElement>("stageWrap");
  requiredElement("stageArea").addEventListener("pointerdown", (event) => {
    if (shell.workspace.kind === "timeline" && event.target === event.currentTarget) setSelected(null);
  });
  function selectedHit(): PlayerHitRect | null {
    return previewAvailable && selectedItemId
      ? topHitForClip(player.hitRects ?? [], selectedItemId)
      : null;
  }
  function positionSelectionBox(): void {
    selectionBox.style.transform = "";
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
    if (shell.workspace.kind !== "timeline" || !previewAvailable || !event.isPrimary) return;
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
    const offsetX = (position.x - canvasDrag.baseX) * rect.width;
    const offsetY = (position.y - canvasDrag.baseY) * rect.height;
    positionSelectionBox();
    selectionBox.style.transform = `translate(${offsetX}px, ${offsetY}px)`;
    showDragTip(event.clientX, event.clientY, `${Math.round(position.x * documentView.canvas.width)}, ${Math.round(position.y * documentView.canvas.height)} px`);
  });
  playerCanvas.addEventListener("pointerup", (event) => {
    const drag = canvasDrag;
    canvasDrag = null;
    selectionBox.style.transform = ""; hideDragTip();
    if (!drag || event.pointerId !== drag.pointerId || !drag.moved) return;
    void queueWorkingCopy(() => editTimelineClip(workingCopy, drag.timelinePath, (target) => {
      if (target.band !== "visual") throw new Error("selected item is not a visual clip");
      target.clip.position = [drag.lastX, drag.lastY];
    }));
  });
  playerCanvas.addEventListener("pointercancel", () => {
    canvasDrag = null;
    selectionBox.style.transform = "";
    hideDragTip();
    positionSelectionBox();
  });
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      canvasDrag = null;
      selectionBox.style.transform = "";
      timelineTrim = null;
      sequenceDragPath = null;
      hideDragTip();
      positionSelectionBox();
    }
  });
  window.addEventListener("resize", positionSelectionBox);

  inspector.addEventListener("studio-inspector-intent", (event) => {
    const intent = (event as CustomEvent<StudioInspectorIntent>).detail;
    if (intent.type === "edit") {
      void editItem(intent.clipId, intent.key, intent.value).then(renderInspector);
    } else if (intent.type === "delete") {
      void deleteItem(intent.clipId);
    } else if (intent.type === "open-motion") {
      void openMotionClip(intent.clipId).catch((error) => {
        rendererMessage = error instanceof Error ? error.message : String(error);
        renderMeta();
      });
    }
  });

  let externalUpdatePending = false;
  let externalRevision = 0;

  function consumeExternalUpdate(): void {
    if (!externalUpdatePending) return;
    externalUpdatePending = false;
    if (externalRevision <= baseRevision) return;
    if (dirty || conflictMessage !== null) {
      if (conflictMessage === null) {
        setConflict(`External ${storageKind} update; keep this draft or reload the saved content`);
      }
    } else {
      void reloadProject();
    }
  }

  async function saveOneSnapshot(): Promise<boolean> {
    if (!saveTimeline) throw new Error("Studio save capability is unavailable");
    const localSnapshotJson = JSON.stringify(workingCopy);
    const submitted = captureTimelineSaveSnapshot(workingCopy);
    const response = await saveTimeline({
      baseRevision,
      timeline: submitted.timeline,
      intent: "studio save",
    });
    if (response.outcome === "staleBase") {
      setConflict(
        storageKind === "file" ? "The JSON file changed externally; reload explicitly before saving again" : `stale base · HEAD is revision ${response.revision}; reload explicitly before saving again`,
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
    baseRevision = next.timelineRevision.revision;
    savedSnapshotJson = JSON.stringify(committed);
    const unchanged = JSON.stringify(workingCopy) === localSnapshotJson;
    if (unchanged) workingCopy = structuredClone(committed);
    markDirty();
    setConflict(null);
    renderWorkingCopy();
    renderMeta();
    return true;
  }

  const saveQueue = new TimelineSaveQueue(saveOneSnapshot, consumeExternalUpdate);

  async function requestSave(): Promise<boolean> {
    if (!canSaveTimeline || !dirty || saving) return false;
    saving = true;
    renderMeta();
    try { return await saveQueue.request(); }
    catch (error) {
      setConflict(`Save failed: ${error instanceof Error ? error.message : String(error)}`);
      return false;
    } finally { saving = false; renderMeta(); }
  }

  async function reloadProject(): Promise<void> {
    try { await reloadSavedSnapshot(); }
    catch (error) {
      setConflict(`Cannot reload ${storageKind}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  async function reloadSavedSnapshot(): Promise<void> {
    const next = await loadTimeline() as StudioConfig;
    draftPreview.invalidate();
    config = next;
    savedSnapshotJson = JSON.stringify(next.timeline);
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
    scheduleDraftPreview();
  }

  projectControls.addEventListener("studio-project-intent", (event) => {
    const intent = (event as CustomEvent<StudioProjectIntent>).detail;
    if (intent.type === "save") void requestSave();
    else if (!dirty || confirm(`Discard unsaved changes and reload the saved ${storageKind}?`)) void reloadProject();
  });

  shell.addEventListener("studio-host-event", (event) => {
    const detail = (event as CustomEvent<{ type: string; data: unknown }>).detail;
    if (detail.type !== "timeline" && detail.type !== "project") return;
    if (isRecord(detail.data) && typeof detail.data.revision === "number" && detail.data.revision <= baseRevision) return;
    externalRevision = isRecord(detail.data) && typeof detail.data.revision === "number" ? detail.data.revision : baseRevision + 1;
    if (saveQueue.active) {
      externalUpdatePending = true;
      return;
    }
    if (dirty) {
      setConflict(`External ${storageKind} update; keep this draft or reload the saved content`);
    } else {
      void reloadProject();
    }
  });

  function currentMotionContext(clipId: string): GoodMotionContext {
    if (!motionProjection) {
      throw new Error("Project Motion context is unavailable");
    }
    const context = projectMotionContextFromAdmittedPreview(
      { ...config, motion: motionProjection },
      boot as typeof boot & { session: Exclude<typeof boot.session, { kind: "motion-file" }> },
      { clipId },
      compiledCopy.timeline,
    );
    if (context.status !== "ok") throw new Error(context.diagnostics.map((item) => item.message).join("\n"));
    const motionFrames = findProjectedItem(compiledCopy.timeline, documentView, clipId)?.motionFrames;
    return motionContextWithTimelineFrames(context, motionFrames);
  }

  async function openMotionClip(clipId: string): Promise<void> {
    if (!options.openMotion || !motionProjection) {
      throw new Error("Motion editing requires a prepared Timeline session");
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
    const returnTime = currentTime();
    const tlScroll = requiredElement<HTMLElement>("timelineScroll");
    const returnScroll = { left: tlScroll.scrollLeft, top: tlScroll.scrollTop };
    player.pause();
    if (animationFrame !== null) cancelAnimationFrame(animationFrame);
    animationFrame = null;
    await player.seek(projected.startSeconds);
    const session: ProjectMotionSession = {
      read: () => ({ context: currentMotionContext(clipId), props: preparedMotionProps(compiledCopy.timeline, motionProjection, clipId, currentMotionContext(clipId).artifact) }),
      clipId,
      context,
      props: preparedMotionProps(compiledCopy.timeline, motionProjection, clipId, context.artifact),
      clipStartS: projected.startSeconds,
      sourceStartS: projected.sourceStartSeconds ?? 0,
      rate: projected.sourceRate ?? 1,
      clipDurationS: projected.durationSeconds,
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
        await flushDraftPreview();
        zoomInput.value = String(zoomScale);
        shell.dispatchIntent({ type: "workspace", workspace: { kind: "timeline" } });
        shell.classList.remove("has-motion-timeline");
        timelinePane.hidden = false;
        inspectorBody.hidden = false;
        motionWorkspaceEl.hidden = true;
        document.getElementById("timelineTitle")!.textContent = "Timeline";
        document.getElementById("timelineHint")!.textContent = "Drag the ruler to seek · Select a clip to edit";
        renderWorkingCopy();
        if (previewAvailable) {
          await player.seek(Math.min(returnTime, player.lastFrameTimeS()));
        }
        tlScroll.scrollLeft = returnScroll.left;
        tlScroll.scrollTop = returnScroll.top;
        syncTransport();
      },
    };
    shell.dispatchIntent({
      type: "workspace",
      workspace: { kind: "motion", clipId, source: clip.source.component },
    });
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

  const layoutObserver = new ResizeObserver(() => {
    if (shell.workspace.kind !== "timeline") return;
    setTimelineZoom(zoomScale); positionSelectionBox();
  });
  layoutObserver.observe(requiredElement("timelineScroll"));
  layoutObserver.observe(requiredElement("stageArea"));
  window.addEventListener("beforeunload", () => layoutObserver.disconnect());
  fitPixelsPerSecond(1);
  renderWorkingCopy();
  setSelected(selectedItemId);
  requiredElement("loading").hidden = true;
  await seek(config.initialTimeS ?? 0);
  syncTransport();
  shell.dispatchIntent({ type: "preview-status", status: previewAvailable ? "ready" : "loading", message: null });
  transport.disabled = !previewAvailable;
  window.addEventListener("beforeunload", (event) => { if (dirty) { event.preventDefault(); event.returnValue = ""; } });
  scheduleDraftPreview();
  shell.addEventListener("studio-open-motion", () => {
    if (selectedItemId) {
      void openMotionClip(selectedItemId).catch((error) => {
        rendererMessage = error instanceof Error ? error.message : String(error);
        renderMeta();
      });
    }
  });
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
