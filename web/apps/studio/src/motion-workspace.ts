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
import type { StudioHeaderMeta, StudioTimeline, StudioTransport, StudioTransportIntent } from "./timeline-components.ts";
import { TimelineEditQueue } from "./timeline-edit-queue.ts";
import type { ProjectMotionEdit } from "./project-motion-edit.ts";

interface MotionValue { kind: string; value: unknown }
interface LocateResult { key: string; location: string; kind?: "object3d" }

export interface ProjectMotionSession {
  clipId: string;
  context: GoodMotionContext;
  props: Record<string, MotionValue>;
  clipStartS: number;
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
  const loadMotion = projectSession
    ? async (): Promise<MotionContext> => projectSession.context
    : host.loadMotion?.bind(host);
  if (!loadMotion) throw new Error("Studio Motion host is unavailable");
  const abort = new AbortController();
  const listenerOptions = { signal: abort.signal };
  const player = requiredElement<VallePlayerElement>("studioPlayer");
  const transport = requiredElement<StudioTransport>("studioTransport");
  const meta = requiredElement<StudioHeaderMeta>("meta");
  const timeline = requiredElement<StudioTimeline>("tracks");
  const timelineInspector = requiredElement<HTMLElement>("timelineInspector");
  const workspace = requiredElement<StudioMotionWorkspace>("motionWorkspace");
  const loading = requiredElement<HTMLElement>("loading");
  const errorBox = requiredElement<HTMLElement>("errbox");
  const queue = new TimelineEditQueue();
  timeline.hidden = true;
  timelineInspector.hidden = true;
  workspace.hidden = false;

  let context: GoodMotionContext | null = null;
  let props: Record<string, MotionValue> = structuredClone(projectSession?.props ?? {});
  let timing: GoodMotionContext["timing"] | null = null;
  let cues: GoodMotionContext["cueBindings"] = {};
  let lastLocate: LocateResult | null = null;

  const renderWorkspace = (next: MotionContext): void => {
    const canReturn = projectSession != null;
    if (next.status === "error") {
      shell.dispatchIntent({ type: "diagnostics", messages: next.diagnostics.map((item) => item.message) });
      workspace.renderWorkspace({
        input: next.input,
        generation: next.generation,
        status: "error",
        props: [],
        data: [],
        phases: [],
        cues: [],
        diagnostics: next.diagnostics.map((item) => ({
          label: `[${item.class}:${item.code}] ${item.message}`,
          location: item.span ? `${item.sourcePath ?? next.input}:${item.span.line}:${item.span.column}` : item.sourcePath ?? next.input,
        })),
        mappings: [],
        canReturn,
      });
      return;
    }
    workspace.renderWorkspace(buildMotionWorkspaceModel(next, props, timing ?? next.timing, cues, canReturn));
  };

  const applyGoodContext = async (
    next: GoodMotionContext,
    resetControls: boolean,
    openRenderPackage: boolean,
  ): Promise<void> => {
    context = next;
    if (resetControls) {
      props = structuredClone(projectSession?.props ?? {});
      timing = structuredClone(next.timing);
      cues = structuredClone(next.cueBindings);
    }
    if (!projectSession && openRenderPackage) {
      const preview = buildMotionPreview(next);
      player.style.aspectRatio = `${next.viewport.width} / ${next.viewport.height}`;
      if (player.state === "ready") await player.replaceRenderPackage(preview);
      else await player.load(preview);
      await player.seek(Math.min(player.currentTime(), player.lastFrameTimeS()));
    }
    const workspaceDurationS = next.durationFrames * next.fps.den / next.fps.num;
    transport.configure(workspaceDurationS, next.fps.num / next.fps.den);
    syncTransport();
    meta.chips = [
      { text: `${next.viewport.width}×${next.viewport.height}`, strong: true },
      { text: `${next.fps.num / next.fps.den} fps`, strong: true },
      { text: `Motion #${next.generation}`, tone: "live" },
    ];
    shell.dispatchIntent({ type: "diagnostics", messages: next.diagnostics.map((item) => item.message) });
    renderWorkspace(next);
    loading.hidden = true;
    errorBox.hidden = true;
  };

  const syncTransport = (): void => {
    if (!context) return;
    const durationS = context.durationFrames * context.fps.den / context.fps.num;
    const timeS = projectSession
      ? Math.max(0, Math.min(durationS, player.currentTime() - projectSession.clipStartS))
      : player.currentTime();
    transport.sync(timeS, durationS, context.fps.num / context.fps.den, player.playing);
  };

  transport.addEventListener("studio-transport-intent", (event) => {
    const intent = (event as CustomEvent<StudioTransportIntent>).detail;
    if (intent.type === "toggle-play") {
      if (player.playing) player.pause();
      else void player.play();
    } else if (intent.type === "pause") {
      if (player.playing) player.pause();
    } else {
      const target = projectSession ? projectSession.clipStartS + intent.timeS : intent.timeS;
      void player.seek(target).then(syncTransport);
    }
  }, listenerOptions);
  player.addEventListener("time", syncTransport, listenerOptions);
  player.addEventListener("play", syncTransport, listenerOptions);

  workspace.addEventListener("motion-workspace-intent", (event) => {
    const intent = (event as CustomEvent<MotionWorkspaceIntent>).detail;
    if (intent.type === "copy-props") {
      const copied = projectSession
        ? Object.fromEntries(Object.entries(props).map(([name, value]) => [name, value.value]))
        : props;
      void navigator.clipboard?.writeText(JSON.stringify(copied, null, 2));
      return;
    }
    if (intent.type === "return") {
      if (projectSession) {
        void projectSession.returnToTimeline().finally(() => abort.abort());
      }
      return;
    }
    if (!context) return;
    if (projectSession) {
      void queue.enqueue(async () => {
        const next = await projectSession.applyEdit(intent);
        context = next.context;
        props = next.props;
        timing = structuredClone(next.context.timing);
        cues = structuredClone(next.context.cueBindings);
        renderWorkspace(next.context);
        syncTransport();
      }, (error) => {
        const message = error instanceof Error ? error.message : String(error);
        shell.dispatchIntent({
          type: "diagnostics",
          messages: [...(context?.diagnostics.map((item) => item.message) ?? []), message],
        });
        if (context) renderWorkspace(context);
      });
      return;
    }
    if (intent.type === "prop") props[intent.name] = intent.value;
    else if (intent.type === "phase" && timing) {
      if (intent.key === "enterFrames") timing.enterFrames = intent.value;
      if (intent.key === "exitFrames") timing.exitFrames = intent.value;
    } else if (intent.type === "cue" && cues[intent.cue]) {
      const cue = cues[intent.cue];
      const minimumRangeFrames = Math.max(1, cue.enterFrames + cue.exitFrames);
      if (intent.key === "startFrame") {
        cue.startFrame = Math.max(0, Math.min(intent.value, cue.endFrame - minimumRangeFrames));
      }
      if (intent.key === "endFrame") {
        cue.endFrame = Math.min(
          context.durationFrames,
          Math.max(intent.value, cue.startFrame + minimumRangeFrames),
        );
      }
      const rangeFrames = Math.max(0, cue.endFrame - cue.startFrame);
      if (intent.key === "enterFrames") {
        const max = rangeFrames - cue.exitFrames;
        cue.enterFrames = Math.max(0, Math.min(intent.value, max));
      }
      if (intent.key === "exitFrames") {
        const max = rangeFrames - cue.enterFrames;
        cue.exitFrames = Math.max(0, Math.min(intent.value, max));
      }
    }
    void queue.enqueue(() => applyGoodContext(context!, false, false));
  }, listenerOptions);

  const locateAt = async (clientX: number, clientY: number): Promise<LocateResult | null> => {
    if (!context) return null;
    const canvas = player.canvas;
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    const x = ((clientX - rect.left) / rect.width) * context.viewport.width;
    const y = ((clientY - rect.top) / rect.height) * context.viewport.height;
    const inspection = await player.motionInspection(
      projectSession?.clipId ?? "clip:motion", x, y,
    );
    if (inspection.object) {
      const mapping = context.sourceMap.objects.find((item) => item.semanticAddress === inspection.object?.semanticAddress);
      return mapping ? mappingResult(context.input, inspection.object.semanticAddress, mapping, "object3d") : {
        key: inspection.object.semanticAddress,
        location: context.input,
        kind: "object3d",
      };
    }
    let hit: { key: string; area: number } | null = null;
    for (const [key, [bx, by, bw, bh]] of Object.entries(inspection.boxes)) {
      if (x < bx || y < by || x > bx + bw || y > by + bh) continue;
      const area = bw * bh;
      if (!hit || area < hit.area) hit = { key, area };
    }
    if (!hit) return null;
    const mapping = context.sourceMap.nodes.find((item) => item.key === hit?.key);
    return mapping ? mappingResult(context.input, hit.key, mapping) : { key: hit.key, location: context.input };
  };

  await player.updateComplete;
  player.canvas?.addEventListener("click", (event) => {
    void locateAt(event.clientX, event.clientY).then((located) => {
      lastLocate = located;
      if (lastLocate) void navigator.clipboard?.writeText(lastLocate.location);
    });
  }, listenerOptions);

  let lastHostUpdate: Promise<void> = Promise.resolve();
  if (!projectSession) shell.addEventListener("studio-host-event", (event) => {
    const hostEvent = (event as CustomEvent<{ type: string }>).detail;
    if (hostEvent.type === "motion") {
      lastHostUpdate = loadMotion({}).then(async (next) => {
        renderWorkspace(next);
        if (next.status === "ok") await queue.enqueue(() => applyGoodContext(next, true, true));
      });
    } else if (hostEvent.type === "runtime") {
      location.reload();
    }
  }, listenerOptions);

  const initial = await loadMotion({});
  renderWorkspace(initial);
  if (initial.status === "ok") await applyGoodContext(initial, true, true);
  else loading.hidden = true;

  const search = new URLSearchParams(location.search);
  if (!projectSession && search.has("hot-smoke") && initial.status === "ok") {
    await runMotionHotSmoke(initial, player, workspace, shell, () => lastHostUpdate);
  } else if (!projectSession && search.has("smoke") && initial.status === "ok") {
    await runMotionSmoke(initial, player, workspace, () => props, (next) => { props = next; }, () => timing!, (next) => { timing = next; }, () => locateAtCenter(player, locateAt), () => lastLocate);
  }
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
      kind: kind === "number" || kind === "bool" || kind === "string" || kind === "select" ? kind : "readonly",
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
    return { key, label: key === "enterFrames" ? "enter" : "exit", value: timing[key], min: Number(schema.min ?? 0), max: Number(schema.max ?? context.durationFrames) };
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
  workspace: StudioMotionWorkspace,
  getProps: () => Record<string, MotionValue>,
  setProps: (props: Record<string, MotionValue>) => void,
  getTiming: () => GoodMotionContext["timing"],
  setTiming: (timing: GoodMotionContext["timing"]) => void,
  locateCenter: () => Promise<LocateResult | null>,
  getLastLocate: () => LocateResult | null,
): Promise<void> {
  const before = (await player.captureFrame(0)).png;
  const controls = motionControls(context);
  const prop = Object.entries(record(controls.props)).find(([, value]) => record(record(value).control).kind === "number");
  let propsEditable = false;
  let fixedPackagePinnedAfterLocalEdit = true;
  let applied: number | undefined;
  if (prop) {
    propsEditable = true;
    const [name, raw] = prop;
    const control = record(record(raw).control);
    const current = Number(record(record(raw).default).value ?? 0);
    const min = typeof control.min === "number" ? control.min : Number.NEGATIVE_INFINITY;
    const max = typeof control.max === "number" ? control.max : Number.POSITIVE_INFINITY;
    const candidate = current === 0 ? (Number.isFinite(max) ? max : 1) : 0;
    const target = Math.min(max, Math.max(min, candidate));
    setProps({ ...getProps(), [name]: { kind: "number", value: target } });
    const after = (await player.captureFrame(0)).png;
    fixedPackagePinnedAfterLocalEdit = bytesEqual(before, after);
    applied = target;
  }
  const defaultEnter = context.timing.enterFrames;
  setTiming({ ...getTiming(), enterFrames: Math.min(context.durationFrames - 1, defaultEnter + 5) });
  // The local timing edit above is authoring state only; the pinned render must keep executing
  // the producer-owned phase window. Sample inside that original window instead of relying on the
  // unsaved edit to extend it.
  const enterFrame = defaultEnter > 0
    ? Math.min(context.durationFrames - 1, Math.max(0, defaultEnter - 1))
    : 0;
  const steadyFrame = Math.min(context.durationFrames - 1, 60);
  const enter = (await player.captureFrame(enterFrame * context.fps.den / context.fps.num)).png;
  const steady = (await player.captureFrame(steadyFrame * context.fps.den / context.fps.num)).png;
  const playbackVariesAcrossFrames = !bytesEqual(enter, steady);
  // Source-location inspection is defined for an active Motion operation. Return to the first
  // frame after the temporal probe instead of inheriting its terminal seek position.
  await player.seek(0);
  const locate = await locateCenter() ?? getLastLocate();
  await fetch("/result", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      status: "ok",
      implementation: "valle-motion-studio",
      userAgent: navigator.userAgent,
      stats: {
        userAgent: navigator.userAgent,
        motionStudio: {
          propsEditable,
          fixedPackagePinnedAfterLocalEdit,
          playbackVariesAcrossFrames,
          locate,
          prop: prop?.[0],
          applied,
        },
      },
      captures: [],
    }),
  });
  workspace.renderWorkspace(buildMotionWorkspaceModel(context, getProps(), getTiming(), context.cueBindings, false));
}

function locateAtCenter(player: VallePlayerElement, locate: (x: number, y: number) => Promise<LocateResult | null>): Promise<LocateResult | null> {
  const rect = player.canvas?.getBoundingClientRect();
  return rect ? locate(rect.left + rect.width / 2, rect.top + rect.height / 2) : Promise.resolve(null);
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
