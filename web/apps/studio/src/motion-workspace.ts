import { DraftPreview } from "./draft-preview.ts";
import { createTimelineCompilerRuntime } from "@valle/player-core";
import { playerRuntimeAssetsFromStudioBoot, type MotionPreviewRequest } from "./host.ts";
import type { VallePlayerElement } from "@valle/player";

import { motionControls, type MotionContext, type StudioHost } from "./host.ts";
import type {
  MotionHandleView,
  MotionMappingView,
  MotionPropControlView,
  MotionWorkspaceIntent,
  MotionWorkspaceViewModel,
  StudioMotionWorkspace,
} from "./motion-components.ts";
import { buildMotionPreview, type GoodMotionContext } from "./motion-preview.ts";
import type { ValleStudioApp } from "./studio-shell.ts";
import type { StudioTimeline, StudioTransport, StudioTransportIntent } from "./timeline-components.ts";
import { TimelineEditQueue } from "./timeline-edit-queue.ts";
import type { ProjectMotionEdit } from "./project-motion-edit.ts";

interface MotionValue { kind: string; value: unknown }
interface LocateResult { key: string; location: string; kind?: "object3d" }

export interface ProjectMotionSession {
  read(): { context: GoodMotionContext; props: Record<string, MotionValue> };
  clipId: string;
  context: GoodMotionContext;
  props: Record<string, MotionValue>;
  clipStartS: number;
  sourceStartS: number;
  rate: number;
  clipDurationS: number;
  applyEdit(edit: ProjectMotionEdit): Promise<{
    context: GoodMotionContext;
    props: Record<string, MotionValue>;
  }>;
  returnToTimeline(): Promise<void>;
}

export async function startMotionStudio(
  shell: ValleStudioApp,
  host: StudioHost,
  projectSession?: ProjectMotionSession,
): Promise<void> {
  const loadMotion = projectSession ? async () => projectSession.context : host.loadMotion?.bind(host);
  if (!loadMotion) throw new Error("Studio Motion host is unavailable");
  const abort = new AbortController();
  const listenerOptions = { signal: abort.signal };
  const player = requiredElement<VallePlayerElement>("studioPlayer");
  const transport = requiredElement<StudioTransport>("studioTransport");
  const timeline = requiredElement<StudioTimeline>("tracks");
  const workspace = requiredElement<StudioMotionWorkspace>("motionWorkspace");
  const loading = requiredElement<HTMLElement>("loading");
  const timelinePane = requiredElement<HTMLElement>("timelinePane");
  const scroll = requiredElement<HTMLElement>("timelineScroll");
  const queue = new TimelineEditQueue();
  const compiler = await createTimelineCompilerRuntime({ runtimeAssets: playerRuntimeAssetsFromStudioBoot(host.boot), runtimeBaseUrl: location.href });
  requiredElement<HTMLElement>("inspectorBody").hidden = true;
  requiredElement<HTMLElement>("selBox").hidden = true;
  workspace.hidden = false;
  shell.dispatchIntent({ type: "workspace", workspace: { kind: "motion", clipId: projectSession?.clipId, source: projectSession?.context.input ?? host.boot.session.kind } });

  let context: GoodMotionContext | null = null;
  let props: Record<string, MotionValue> = structuredClone(projectSession?.props ?? {});
  let timing = { enterFrames: 0, exitFrames: 0 };
  let cues: GoodMotionContext["cueBindings"] = {};
  let lastLocate: LocateResult | null = null;
  let baseline = "";
  const undo: string[] = [], redo: string[] = [];
  let zoom = 1;
  let laneWidth = 1;
  let raf: number | null = null;
  let hostVersion = 0;
  let latestGeneration = -1;
  const snapshot = () => JSON.stringify({ props, timing, cues });
  const frameTime = (frame: number) => context ? compiler.timelineTimeFromFrames(frame, `${context.fps.num}/${context.fps.den}`) : 0;
  const localTime = () => Math.max(0, (projectSession?.sourceStartS ?? 0) + (player.currentTime() - (projectSession?.clipStartS ?? 0)) * (projectSession?.rate ?? 1));
  const syncTransport = () => {
    if (!context || abort.signal.aborted || shell.workspace.kind !== "motion") return;
    const time = Math.min(localTime(), frameTime(Math.max(0, context.durationFrames - 1)));
    transport.sync(time, frameTime(context.durationFrames), context.fps.num / context.fps.den, player.playing);
    const x = time / Math.max(1e-9, frameTime(context.durationFrames)) * laneWidth;
    const line = timeline.querySelector<HTMLElement>("#tlPlayhead");
    const grip = timeline.querySelector<HTMLElement>("#rulerGrip");
    if (line) line.style.left = `${152 + x}px`;
    if (grip) grip.style.left = `${x - 5}px`;
  };
  const seekFrame = async (frame: number) => {
    if (!context || player.state !== "ready") return;
    player.pause();
    const sourceTime = frameTime(Math.max(0, Math.min(context.durationFrames - 1, Math.round(frame))));
    const delta = (sourceTime - (projectSession?.sourceStartS ?? 0)) / (projectSession?.rate ?? 1);
    await player.seek((projectSession?.clipStartS ?? 0) + Math.max(0, Math.min(delta, projectSession ? projectSession.clipDurationS - frameTime(1) : frameTime(context.durationFrames - 1))));
    syncTransport();
  };
  const poll = () => {
    if (raf !== null || abort.signal.aborted) return;
    const tick = () => {
      raf = null;
      if (abort.signal.aborted || !context) return;
      syncTransport();
      const atEnd = projectSession ? player.currentTime() >= projectSession.clipStartS + projectSession.clipDurationS - frameTime(1) - 1e-6 : localTime() >= frameTime(context.durationFrames - 1) - 1e-6;
      if (atEnd) {
        const wasPlaying = player.playing || transport.playing;
        player.pause();
        if (transport.looping && wasPlaying) void seekFrame(0).then(() => player.play()).then(poll);
        else syncTransport();
      } else if (player.playing) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
  };
  const renderTimeline = () => {
    if (!context) return;
    const hasContent = timing.enterFrames > 0 || timing.exitFrames > 0 || Object.keys(cues).length > 0;
    shell.classList.toggle("has-motion-timeline", hasContent);
    timelinePane.hidden = !hasContent;
    if (!hasContent) return;
    laneWidth = Math.max(1, scroll.clientWidth - 152) * zoom;
    const width = (frames: number) => frames / context!.durationFrames * laneWidth;
    const clip = (id: string, label: string, start: number, end: number, editable = true) => ({
      id, kind: "phase", fixedStart: id === "phase-enter", fixedEnd: id === "phase-exit", leftPx: width(start), widthPx: width(Math.max(0, end - start)),
      title: `${label} · ${start}–${end} f`, label, selected: false, timingEditable: editable,
      readOnly: !editable,
    });
    const enter = timing.enterFrames, exitStart = context.durationFrames - timing.exitFrames;
    const tickStep = Math.max(1, Math.ceil(context.durationFrames / Math.max(1, Math.floor(laneWidth / 80))));
    timeline.renderTimeline({
      widthPx: 152 + laneWidth, laneWidthPx: laneWidth, labelWidthPx: 152, playheadLeftPx: 152,
      ticks: Array.from({ length: Math.floor(context.durationFrames / tickStep) + 1 }, (_, i) => ({ leftPx: width(i * tickStep), label: `${i * tickStep} f` })),
      tracks: [{ id: "phases", kind: "phase", name: "Phases", clips: [
        ...(enter > 0 ? [clip("phase-enter", "Enter", 0, enter)] : []),
        ...(exitStart > enter ? [clip("phase-hold", "Hold", enter, exitStart, false)] : []),
        ...(timing.exitFrames > 0 ? [clip("phase-exit", "Exit", exitStart, context.durationFrames)] : []),
      ] }, ...Object.entries(cues).map(([name, cue]) => ({ id: `cue:${name}`, kind: "phase", name,
        clips: [clip(`cue:${name}`, name, cue.startFrame, cue.endFrame)],
      }))],
    });
    syncTransport();
  };
  const render = () => {
    if (!context || abort.signal.aborted) return;
    workspace.renderWorkspace({ ...buildMotionWorkspaceModel(context, props, timing, cues, Boolean(projectSession)), selectedLocation: lastLocate?.location ?? null });
    renderTimeline();
    document.getElementById("timelineTitle")!.textContent = "Phases & cues";
    document.getElementById("trackCount")!.textContent = "";
    document.getElementById("timelineSelection")!.textContent = "";
    document.getElementById("timelineHint")!.textContent = "Drag the ruler to seek · Adjust phase and cue boundaries";
    if (!projectSession) {
      shell.dispatchIntent({ type: "dirty", value: snapshot() !== baseline });
      shell.dispatchIntent({ type: "history", canUndo: undo.length > 0, canRedo: redo.length > 0 });
    }
  };
  const draftPreview = new DraftPreview<MotionPreviewRequest, MotionContext>({
    prepare: (draft) => {
      if (!host.prepareMotionPreview) throw new Error("Motion draft preview is unavailable");
      return host.prepareMotionPreview(draft);
    },
    apply: async (next, _draft, isCurrent) => {
      if (next.status === "error") throw new Error(next.diagnostics.map((d) => d.message).join("\n"));
      await player.replaceRenderPackage({ ...buildMotionPreview(next), isCurrent });
      if (isCurrent()) context = next;
    },
    updating: () => shell.dispatchIntent({ type: "preview-status", status: "updating", message: null }),
    ready: () => { shell.dispatchIntent({ type: "preview-status", status: "ready", message: null }); syncTransport(); },
    failed: (error) => shell.dispatchIntent({ type: "preview-status", status: "error", message: error instanceof Error ? error.message : String(error) }),
  });
  const schedule = () => draftPreview.schedule({
    props: Object.fromEntries(Object.entries(props).map(([name, value]) => [name, value.value])), timing,
    cues: Object.fromEntries(Object.entries(cues).map(([name, { type: _type, ...cue }]) => [name, cue])),
  });
  const applyEdit = async (intent: ProjectMotionEdit) => {
    if (!context || abort.signal.aborted) return;
    if (projectSession) {
      const next = await projectSession.applyEdit(intent);
      if (abort.signal.aborted) return;
      context = next.context; props = next.props; timing = structuredClone(context.timing); cues = structuredClone(context.cueBindings);
    } else {
      const before = snapshot();
      if (intent.type === "prop") props[intent.name] = intent.value;
      if (intent.type === "phase") {
        const value = Math.max(0, Math.min(context.durationFrames - (intent.key === "enterFrames" ? timing.exitFrames : timing.enterFrames), intent.value));
        timing = { ...timing, [intent.key]: value };
      }
      if (intent.type === "cue" && cues[intent.cue]) {
        const cue = cues[intent.cue]!;
        const handle = buildMotionWorkspaceModel(context, props, timing, cues, false).cues.find((h) => h.cue === intent.cue && h.key === intent.key)!;
        cues = { ...cues, [intent.cue]: { ...cue, [intent.key]: Math.max(handle.min, Math.min(handle.max, intent.value)) } };
      }
      if (snapshot() !== before) { undo.push(before); if (undo.length > 40) undo.shift(); redo.length = 0; schedule(); }
    }
    render();
  };
  const enqueueEdit = (intent: ProjectMotionEdit) => void queue.enqueue(() => applyEdit(intent), (error) => {
    shell.dispatchIntent({ type: "preview-status", status: "error", message: error instanceof Error ? error.message : String(error) }); render();
  });
  const returnToTimeline = async () => {
    if (!projectSession || abort.signal.aborted) return;
    await queue.idle();
    abort.abort(); draftPreview.invalidate();
    if (raf !== null) cancelAnimationFrame(raf);
    player.pause();
    await projectSession.returnToTimeline();
  };
  workspace.addEventListener("motion-workspace-intent", (event) => {
    const intent = (event as CustomEvent<MotionWorkspaceIntent>).detail;
    if (intent.type === "return") { void returnToTimeline(); return; }
    if (intent.type === "copy-props") {
      const model = context ? buildMotionWorkspaceModel(context, props, timing, cues, Boolean(projectSession)) : null;
      void navigator.clipboard.writeText(JSON.stringify(Object.fromEntries(model?.props.map((prop) => [prop.name, prop.value]) ?? []), null, 2)); return;
    }
    if (intent.type !== "prop-end") enqueueEdit(intent);
  }, listenerOptions);
  transport.addEventListener("studio-transport-intent", (event) => {
    if (!context || player.state !== "ready") return;
    const intent = (event as CustomEvent<StudioTransportIntent>).detail;
    if (intent.type === "toggle-play") { if (player.playing) { player.pause(); syncTransport(); } else void player.play().then(poll); }
    else if (intent.type === "play") void player.play().then(poll);
    else if (intent.type === "pause") { player.pause(); syncTransport(); }
    else if (intent.type === "first-frame") void seekFrame(0);
    else if (intent.type === "last-frame") void seekFrame(context.durationFrames - 1);
    else if (intent.type === "seek-frame") void seekFrame(intent.frame);
    else if (intent.type === "seek") void seekFrame(intent.timeS * context.fps.num / context.fps.den);
    else if (intent.type === "mute") player.setMuted(intent.value);
  }, listenerOptions);
  player.addEventListener("time", () => { syncTransport(); requiredElement<HTMLElement>("selBox").hidden = true; }, listenerOptions);
  shell.addEventListener("studio-return-timeline", () => void returnToTimeline(), listenerOptions);
  shell.addEventListener("studio-preview-retry", () => { if (!projectSession) { schedule(); void draftPreview.flush(); } }, listenerOptions);
  shell.addEventListener("studio-draft-updated", () => {
    if (!projectSession) return;
    const next = projectSession.read(); context = next.context; props = next.props;
    timing = structuredClone(context.timing); cues = structuredClone(context.cueBindings); render();
  }, listenerOptions);
  shell.addEventListener("studio-history-intent", (event) => {
    if (projectSession) return;
    const action = (event as CustomEvent<{ type: string }>).detail.type;
    const source = action === "undo" ? undo : redo, target = action === "undo" ? redo : undo;
    const previous = source.pop(); if (!previous) return;
    target.push(snapshot()); ({ props, timing, cues } = JSON.parse(previous)); render(); schedule();
  }, listenerOptions);
  window.addEventListener("beforeunload", (event) => {
    if (!projectSession && snapshot() !== baseline) { event.preventDefault(); event.returnValue = ""; }
  }, listenerOptions);
  for (const id of ["fitTimeline", "zoomIn", "zoomOut", "timelineZoom"]) {
    document.getElementById(id)?.addEventListener(id === "timelineZoom" ? "input" : "click", () => {
      zoom = id === "fitTimeline" ? 1 : id === "timelineZoom" ? Number((document.getElementById(id) as HTMLInputElement).value)
        : Math.max(0.25, Math.min(8, zoom + (id === "zoomIn" ? 0.25 : -0.25)));
      (document.getElementById("timelineZoom") as HTMLInputElement).value = String(zoom); renderTimeline();
    }, listenerOptions);
  }
  const observer = new ResizeObserver(renderTimeline); observer.observe(scroll);
  abort.signal.addEventListener("abort", () => observer.disconnect(), { once: true });
  let gesture: { id: string | null; edge: string | undefined; x: number; pointer: number } | null = null;
  timeline.addEventListener("pointerdown", (event) => {
    const target = event.target as Element;
    const ruler = target.closest<HTMLElement>(".ruler-lane");
    const handle = target.closest<HTMLElement>(".trim");
    if (event.button !== 0 || (!ruler && !handle)) return;
    const id = handle?.closest<HTMLElement>("[data-clip-id]")?.dataset.clipId ?? null;
    gesture = { id, edge: handle?.dataset.edge, x: event.clientX, pointer: event.pointerId };
    timeline.setPointerCapture(event.pointerId); event.preventDefault();
    if (ruler && context) void seekFrame((event.clientX - ruler.getBoundingClientRect().left) / laneWidth * context.durationFrames);
  }, listenerOptions);
  timeline.addEventListener("pointermove", (event) => {
    if (!gesture || !context) return;
    if (!gesture.id) {
      const ruler = timeline.querySelector<HTMLElement>(".ruler-lane")!;
      void seekFrame((event.clientX - ruler.getBoundingClientRect().left) / laneWidth * context.durationFrames);
    }
  }, listenerOptions);
  timeline.addEventListener("pointerup", (event) => {
    const active = gesture; gesture = null;
    if (!active || !context) return;
    if (timeline.hasPointerCapture(event.pointerId)) timeline.releasePointerCapture(event.pointerId);
    const delta = Math.round((event.clientX - active.x) / laneWidth * context.durationFrames);
    if (!active.id || delta === 0) return;
    if (active.id === "phase-enter") enqueueEdit({ type: "phase", key: "enterFrames", value: timing.enterFrames + delta });
    else if (active.id === "phase-exit") enqueueEdit({ type: "phase", key: "exitFrames", value: timing.exitFrames - delta });
    else if (active.id.startsWith("cue:")) {
      const cue = active.id.slice(4), key = active.edge === "left" ? "startFrame" : "endFrame";
      enqueueEdit({ type: "cue", cue, key, value: cues[cue]![key] + delta });
    }
  }, listenerOptions);
  timeline.addEventListener("pointercancel", () => { gesture = null; }, listenerOptions);
  window.addEventListener("keydown", (event) => { if (event.key === "Escape") gesture = null; }, listenerOptions);
  await player.updateComplete;
  player.canvas?.addEventListener("click", (event) => {
    if (!context || player.state !== "ready") return;
    const active = context;
    player.pause();
    const rect = player.canvas!.getBoundingClientRect();
    const x = (event.clientX - rect.left) / rect.width * active.viewport.width;
    const y = (event.clientY - rect.top) / rect.height * active.viewport.height;
    void player.motionInspection(projectSession?.clipId ?? "clip:motion", x, y).then((inspection) => {
      const hit = Object.entries(inspection.boxes).filter(([, [bx, by, bw, bh]]) => x >= bx && y >= by && x <= bx + bw && y <= by + bh)
        .sort((a, b) => a[1][2] * a[1][3] - b[1][2] * b[1][3])[0];
      const mapping = hit ? active.sourceMap.nodes.find((item) => item.key === hit[0]) : null;
      const objectMapping = inspection.object ? active.sourceMap.objects.find((item) => item.key === inspection.object!.semanticAddress) : null;
      lastLocate = objectMapping && inspection.object ? mappingResult(active.input, inspection.object.semanticAddress, objectMapping, "object3d")
        : hit ? mapping ? mappingResult(active.input, hit[0], mapping) : { key: hit[0], location: active.input } : null;
      const selection = requiredElement<HTMLElement>("selBox");
      selection.hidden = !hit;
      if (hit) {
        const parent = selection.offsetParent?.getBoundingClientRect() ?? rect;
        const [bx, by, bw, bh] = hit[1];
        selection.style.cssText = `left:${rect.left - parent.left + bx / active.viewport.width * rect.width}px;top:${rect.top - parent.top + by / active.viewport.height * rect.height}px;width:${bw / active.viewport.width * rect.width}px;height:${bh / active.viewport.height * rect.height}px`;
      }
      render();
    }).catch(() => { lastLocate = null; render(); });
  }, listenerOptions);
  const install = async (next: MotionContext) => {
    if (abort.signal.aborted || next.generation < latestGeneration) return;
    latestGeneration = next.generation;
    if (next.status === "error") {
      shell.dispatchIntent({ type: "preview-status", status: "error", message: next.diagnostics.map((d) => d.message).join("\n") });
      if (workspace.viewModel) workspace.renderWorkspace({ ...workspace.viewModel, status: "error", generation: next.generation,
        diagnostics: next.diagnostics.map((item) => ({ label: item.message, location: "" })) });
      loading.hidden = true; return;
    }
    context = next;
    shell.dispatchIntent({ type: "workspace", workspace: { kind: "motion", clipId: projectSession?.clipId, source: next.input } });
    props = structuredClone(projectSession?.props ?? initialMotionProps(next));
    timing = structuredClone(next.timing); cues = structuredClone(next.cueBindings); baseline = snapshot();
    if (!projectSession) {
      const preview = buildMotionPreview(next);
      if (player.state === "ready") await player.replaceRenderPackage(preview); else await player.load(preview);
    }
    shell.style.setProperty("--canvas-aspect", String(next.viewport.width / next.viewport.height));
    player.style.aspectRatio = `${next.viewport.width} / ${next.viewport.height}`;
    shell.style.setProperty("--canvas-width", `${next.viewport.width}px`);
    shell.style.setProperty("--canvas-height", `${next.viewport.height}px`);
    transport.disabled = false; player.setMuted(transport.muted);
    shell.dispatchIntent({ type: "preview-status", status: "ready", message: null });
    document.getElementById("stageMeta")!.textContent = `${next.viewport.width} × ${next.viewport.height} · ${next.fps.num / next.fps.den} fps`;
    loading.hidden = true; requiredElement<HTMLElement>("errbox").hidden = true; render(); syncTransport();
  };
  if (!projectSession) shell.addEventListener("studio-host-event", (event) => {
    const hostEvent = (event as CustomEvent<{ type: string }>).detail;
    if (hostEvent.type !== "motion") return;
    const version = ++hostVersion;
    if (snapshot() !== baseline) { shell.dispatchIntent({ type: "conflict", value: true, message: "Source changed outside Studio. Your temporary parameters are kept." }); return; }
    draftPreview.invalidate();
    void loadMotion({}).then((next) => { if (version === hostVersion) return install(next); });
  }, listenerOptions);
  shell.addEventListener("studio-discard-draft", () => {
    if (projectSession || !confirm("Discard temporary parameters and reload the source?")) return;
    draftPreview.invalidate(); undo.length = 0; redo.length = 0;
    void loadMotion({}).then(install).then(() => shell.dispatchIntent({ type: "conflict", value: false, message: null }));
  }, listenerOptions);
  await install(await loadMotion({}));
  const search = new URLSearchParams(location.search);
  if (!projectSession && context) {
    if (search.get("smoke") === "1") await runMotionSmoke(context, player, async (edit) => { await applyEdit(edit); await draftPreview.flush(); });
    else if (search.get("hot-smoke") === "1") await runMotionHotSmoke(context, player, workspace, shell, async () => { await install(await loadMotion({})); });
  }
}

function initialMotionProps(context: GoodMotionContext): Record<string, MotionValue> {
  const source = context.timeline.document.visual.tracks[0]?.items[0];
  if (!source || source.type !== "clip" || source.source.type !== "motion") return {};
  return Object.fromEntries(Object.entries(source.source.props).flatMap(([name, value]) => {
    const parameter = value as unknown as { type: string; value: unknown };
    return parameter.type === "constant" ? [[name, { kind: "value", value: parameter.value }]] : [];
  }));
}

export function buildMotionWorkspaceModel(
  context: GoodMotionContext,
  props: Record<string, MotionValue>,
  timing: GoodMotionContext["timing"],
  cues: GoodMotionContext["cueBindings"],
  canReturn: boolean,
): MotionWorkspaceViewModel {
  const controls = motionControls(context);
  const propSchemas = record(controls.props);
  const dataSchemas = record(controls.data);
  const timingSchemas = record(controls.timing);
  const propViews: MotionPropControlView[] = Object.entries(propSchemas).map(([name, raw]) => {
    const schema = record(raw);
    const control = record(schema.control);
    const current = props[name] ?? record(schema.default);
    const kind = typeof control.kind === "string" ? control.kind : "readonly";
    return {
      name,
      kind: kind === "number" || kind === "bool" || kind === "string" || kind === "select" || kind === "color" ? kind : "readonly",
      value: current.value ?? schema.default ?? "required",
      min: numberOrUndefined(control.min),
      max: numberOrUndefined(control.max),
      step: numberOrUndefined(control.step),
      values: Array.isArray(control.values) ? control.values.map(String) : undefined,
    };
  });
  const preparedData = record(context.preparedData);
  const dataViews = Object.entries(dataSchemas).map(([name, raw]) => {
    const schema = record(raw);
    return {
      name,
      kind: typeof schema.kind === "string" ? schema.kind : "unknown",
      value: preparedData[name],
      maxItems: numberOrUndefined(schema.maxItems),
    };
  });
  const phases: MotionHandleView[] = (["enterFrames", "exitFrames"] as const).map((key) => {
    const schema = record(timingSchemas[key]);
    return { key, label: key === "enterFrames" ? "enter" : "exit", value: timing[key], min: Number(schema.min ?? 0), max: Math.min(Number(schema.max ?? context.durationFrames), context.durationFrames - timing[key === "enterFrames" ? "exitFrames" : "enterFrames"]) };
  });
  const cueViews = Object.entries(cues).flatMap(([cue, binding]) => {
    const rangeFrames = Math.max(0, binding.endFrame - binding.startFrame);
    const minimumRangeFrames = Math.max(1, binding.enterFrames + binding.exitFrames);
    return (["startFrame", "endFrame", "enterFrames", "exitFrames"] as const).map((key) => ({
      cue,
      key,
      label: `${cue}.${key === "startFrame" ? "start" : key === "endFrame" ? "end" : key === "enterFrames" ? "enter" : "exit"}`,
      value: binding[key],
      min: key === "endFrame" ? binding.startFrame + minimumRangeFrames : 0,
      max: key === "startFrame"
        ? Math.max(0, binding.endFrame - minimumRangeFrames)
        : key === "enterFrames"
          ? Math.max(0, rangeFrames - binding.exitFrames)
          : key === "exitFrames"
            ? Math.max(0, rangeFrames - binding.enterFrames)
            : context.durationFrames,
      readOnly: false,
    }));
  });
  const mappings: MotionMappingView[] = [
    ...context.sourceMap.nodes.slice(0, 24).map((item) => mappingView(context.input, item)),
    ...context.sourceMap.objects.slice(0, 24).map((item) => mappingView(context.input, item, "object3d")),
  ].filter((item): item is MotionMappingView => item != null);
  return {
    input: context.input,
    generation: context.generation,
    status: "ok",
    fingerprint: context.artifactDigest,
    props: propViews,
    data: dataViews,
    dataSource: context.dataSource,
    phases,
    cues: cueViews,
    diagnostics: context.diagnostics.map((item) => ({
      label: `[${item.class}:${item.code}] ${item.message}`,
      location: item.span ? `${item.sourcePath ?? context.input}:${item.span.line}:${item.span.column}` : item.sourcePath ?? context.input,
    })),
    mappings,
    canReturn,
  };
}

async function runMotionHotSmoke(
  initial: GoodMotionContext,
  player: VallePlayerElement,
  workspace: StudioMotionWorkspace,
  shell: ValleStudioApp,
  hostUpdate: () => Promise<void>,
): Promise<void> {
  const initialCapture = await player.captureFrame(0);
  const initialPng = initialCapture.png;
  if (!initialPng) throw new Error("Motion hot smoke initial capture is missing PNG bytes");
  let errorProbe: Record<string, unknown> | null = null;
  let busy = false;
  await new Promise<void>((resolve, reject) => {
    const timeout = window.setTimeout(() => {
      shell.removeEventListener("studio-host-event", onHostEvent);
      reject(new Error("Motion hot smoke timed out waiting for error and recovery"));
    }, 30_000);
    const onHostEvent = (): void => {
      if (busy) return;
      busy = true;
      void (async () => {
        await hostUpdate();
        const model = workspace.viewModel;
        if (!model || model.generation <= initial.generation) return;
        if (model.status === "error" && !errorProbe) {
          const errorPng = (await player.captureFrame(player.currentTime())).png;
          errorProbe = {
            generation: model.generation,
            diagnostics: model.diagnostics.length,
            lastGoodFrameKept: bytesEqual(initialPng, errorPng),
          };
          await postMotionHotReport("hot-error-observed", { error: errorProbe });
          return;
        }
        if (model.status === "ok" && errorProbe) {
          const recoveredCapture = await player.captureFrame(player.currentTime());
          const recoveredPng = recoveredCapture.png;
          await postMotionHotReport("ok", {
            initialGeneration: initial.generation,
            initialArtifactDigest: initial.artifactDigest,
            initialRenderId: initialCapture.frame.renderId,
            initialPrograms: initialCapture.stats.programs,
            initialPasses: initialCapture.stats.passes,
            initialProgramCacheHits: initialCapture.stats.programCacheHits,
            initialProgramCacheMisses: initialCapture.stats.programCacheMisses,
            error: errorProbe,
            recoveredGeneration: model.generation,
            recoveredFingerprint: model.fingerprint,
            recoveredRenderId: recoveredCapture.frame.renderId,
            recoveredPrograms: recoveredCapture.stats.programs,
            recoveredPasses: recoveredCapture.stats.passes,
            recoveredProgramCacheHits: recoveredCapture.stats.programCacheHits,
            recoveredProgramCacheMisses: recoveredCapture.stats.programCacheMisses,
            recoveredPixelsChanged: !bytesEqual(initialPng, recoveredPng),
          });
          window.clearTimeout(timeout);
          shell.removeEventListener("studio-host-event", onHostEvent);
          resolve();
        }
      })().catch(reject).finally(() => { busy = false; });
    };
    shell.addEventListener("studio-host-event", onHostEvent);
    void postMotionHotReport("hot-ready", { initialGeneration: initial.generation }).catch(reject);
  });
}

async function postMotionHotReport(status: string, motionHot: Record<string, unknown>): Promise<void> {
  await fetch("/result", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      status,
      implementation: "valle-motion-studio-hot-reload",
      userAgent: navigator.userAgent,
      stats: { userAgent: navigator.userAgent, motionHot },
    }),
  });
}

async function runMotionSmoke(
  context: GoodMotionContext,
  player: VallePlayerElement,
  apply: (edit: ProjectMotionEdit) => Promise<void>,
): Promise<void> {
  const before = (await player.captureFrame(0)).png;
  const prop = Object.entries(record(motionControls(context).props)).find(([, raw]) => record(record(raw).control).kind === "number");
  let draftPreviewUpdated = false;
  let applied: number | undefined;
  if (prop) {
    const [name, raw] = prop;
    const control = record(record(raw).control);
    const current = Number(record(record(raw).default).value ?? 0);
    const min = typeof control.min === "number" ? control.min : -Infinity;
    const max = typeof control.max === "number" ? control.max : Infinity;
    applied = Math.min(max, Math.max(min, current === 0 ? Number.isFinite(max) ? max : 1 : 0));
    await apply({ type: "prop", name, value: { kind: "number", value: applied } });
    draftPreviewUpdated = !bytesEqual(before, (await player.captureFrame(0)).png);
  }
  await apply({ type: "phase", key: "enterFrames", value: Math.min(context.durationFrames - context.timing.exitFrames, context.timing.enterFrames + 5) });
  const enter = (await player.captureFrame(0)).png;
  const steady = (await player.captureFrame(Math.min(context.durationFrames - 1, 60) * context.fps.den / context.fps.num)).png;
  await player.seek(0);
  const inspection = await player.motionInspection("clip:motion", context.viewport.width / 2, context.viewport.height / 2);
  const node = context.sourceMap.nodes.find((item) => typeof item.key === "string" && inspection.boxes[item.key]);
  const locate = node ? mappingResult(context.input, String(node.key), node) : null;
  await fetch("/result", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({
    status: "ok", implementation: "valle-motion-studio", userAgent: navigator.userAgent,
    stats: { userAgent: navigator.userAgent, motionStudio: { propsEditable: Boolean(prop), draftPreviewUpdated,
      playbackVariesAcrossFrames: !bytesEqual(enter, steady), locate, prop: prop?.[0], applied } }, captures: [],
  }) });
}

function mappingView(input: string, item: Record<string, unknown>, kind?: "object3d"): MotionMappingView | null {
  const key = typeof item.key === "string" ? item.key : typeof item.semanticAddress === "string" ? item.semanticAddress : null;
  const span = record(item.span);
  if (!key || typeof span.line !== "number" || typeof span.column !== "number") return null;
  const sourcePath = typeof item.sourcePath === "string" ? item.sourcePath : input;
  return { key, location: `${sourcePath}:${span.line}:${span.column}`, kind };
}

function mappingResult(input: string, key: string, item: Record<string, unknown>, kind?: "object3d"): LocateResult {
  return mappingView(input, { ...item, key }, kind) ?? { key, location: input, kind };
}

function requiredElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing Motion workspace element #${id}`);
  return element as T;
}

function record(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null ? value as Record<string, unknown> : {};
}

function numberOrUndefined(value: unknown): number | undefined {
  return typeof value === "number" ? value : undefined;
}

function bytesEqual(left: Uint8Array | null, right: Uint8Array | null): boolean {
  if (!left || !right || left.length !== right.length) return false;
  return left.every((byte, index) => byte === right[index]);
}
