import {
  VallePlayerElement,
  type CanonicalTimelineDocument,
  type VallePlayerElementOptions,
} from "@valle/player";

  type PlayerOptions = VallePlayerElementOptions;
  type DemoTimeline = CanonicalTimelineDocument["timeline"];
  type DemoConfig = PlayerOptions & {
    timeline: DemoTimeline;
    fps?: unknown;
    components?: { artifacts?: Record<string, unknown> };
    generation?: number;
    initialTimeS?: number;
    playMs?: number;
    captures?: Array<{ timeS: number; sampleId: string }>;
  };

  declare global {
    var vallePlayer: VallePlayerElement | undefined;
  }

  function requiredElement<T extends HTMLElement = HTMLElement>(id: string): T {
    const element = document.getElementById(id);
    if (!element) throw new Error(`missing required demo element #${id}`);
    return element as T;
  }

  const $ = requiredElement;
  const params = new URLSearchParams(location.search);

  function errorText(error: unknown): string {
    if (error instanceof Error) return error.stack ?? error.message;
    return String(error ?? "unknown error");
  }

  function scopedError(scope: string, error: unknown): string {
    return `${scope}: ${errorText(error)}`;
  }

  function errorTarget(target: EventTarget | null): string {
    if (target instanceof HTMLScriptElement) return `script ${target.src || "<inline>"}`;
    if (target instanceof HTMLLinkElement) return `link ${target.href || "<inline>"}`;
    if (target instanceof HTMLImageElement) return `image ${target.currentSrc || target.src}`;
    if (target instanceof HTMLMediaElement) return `media ${target.currentSrc || target.src}`;
    if (target instanceof HTMLElement) return target.outerHTML.slice(0, 240);
    return target === window ? "window" : Object.prototype.toString.call(target);
  }

  function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null;
  }

  async function fetchConfig(): Promise<DemoConfig> {
    const raw: unknown = await (await fetch("/config.json")).json();
    if (
      !isRecord(raw)
      || typeof raw.fixedPackageManifestJson !== "string"
      || typeof raw.timelineJson !== "string"
      || !isRecord(raw.timeline)
      || !isRecord(raw.timeline.document)
      || typeof raw.resourceManifestJson !== "string"
      || typeof raw.verifiedBindingBundleJson !== "string"
      || !isRecord(raw.runtimeAssets)
    ) {
      throw new Error("/config.json must contain one complete fixed package");
    }
    return raw as DemoConfig;
  }

  async function postFailure(message: unknown): Promise<void> {
    await fetch("/result", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ status: "error", message: String(message) }),
    });
  }
  // Visible error reporting; headless probes also receive failures through `postFailure`.
  function showError(message: unknown): void {
    const element = document.getElementById("stage");
    if (element instanceof VallePlayerElement) {
      element.errorMessage = String(message ?? "unknown error");
    }
  }

  window.addEventListener("error", (event) => {
    const playerError = event instanceof CustomEvent && isRecord(event.detail)
      ? event.detail
      : null;
    const location = event instanceof ErrorEvent && event.filename
      ? ` at ${event.filename}:${event.lineno}:${event.colno}`
      : "";
    const target = errorTarget(event.target);
    const scope = typeof playerError?.operation === "string"
      ? `valle-player.${playerError.operation}`
      : "window.error";
    const cause = playerError?.error
      ?? (event instanceof ErrorEvent ? event.error ?? event.message : event);
    const message = `${scopedError(scope, cause)}${location}; target=${target}`;
    showError(message);
    postFailure(message).catch(() => {});
  }, true);
  window.addEventListener("unhandledrejection", (event) => {
    const message = scopedError("window.unhandledrejection", event.reason);
    showError(message);
    postFailure(message).catch(() => {});
  });

  const config = await fetchConfig();
  const player = $<VallePlayerElement>("stage");
  const scrub = $<HTMLInputElement>("scrub");
  const stageWrap = $<HTMLElement>("stageWrap");

  player.configure({
    fixedPackageManifestJson: config.fixedPackageManifestJson,
    timelineJson: config.timelineJson,
    resourceManifestJson: config.resourceManifestJson,
    verifiedBindingBundleJson: config.verifiedBindingBundleJson,
    assets: config.assets,
    assetBaseUrl: "/assets/",
    proxyBase: config.proxyBase ?? null,
    runtimeAssets: config.runtimeAssets,
    runtimeBaseUrl: config.runtimeBaseUrl ?? location.href,
  });
  await player.load();
  globalThis.vallePlayer = player;

  // ── timeline-derived UI state (fps as {num,den} → number; meta chips; scrub range) ──
  let fps = 30;
  function fpsFrom(cfg: DemoConfig): number {
    const [num, den] = cfg.timeline.document.canvas.fps.split("/").map(Number);
    return num! > 0 && den! > 0 ? num! / den! : 30;
  }
  function fmtTime(s: string | number): string {
    s = Math.max(0, Number(s) || 0);
    const m = Math.floor(s / 60);
    const sec = s - m * 60;
    return `${m}:${sec.toFixed(3).padStart(6, "0")}`;
  }
  function frameOf(s: string | number): number { return Math.round((Number(s) || 0) * fps); }

  function chip(html: string, cls = ""): string { return `<span class="chip ${cls}">${html}</span>`; }
  function renderMeta(cfg: DemoConfig): void {
    const cv = cfg.timeline.document.canvas;
    const dur = player.durationS();
    const nAssets = Array.isArray(cfg.assets) ? cfg.assets.length : 0;
    const nComp = Object.keys(cfg.components?.artifacts ?? {}).length;
    const fpsLabel = Number.isInteger(fps) ? `${fps}` : fps.toFixed(2);
    const out: string[] = [];
    if (cv.width && cv.height) out.push(chip(`<b>${cv.width}×${cv.height}</b>`));
    out.push(chip(`<b>${fpsLabel}</b> fps`));
    out.push(chip(`<b>${fmtTime(dur)}</b>`));
    if (nAssets) out.push(chip(`<b>${nAssets}</b> asset${nAssets > 1 ? "s" : ""}`));
    if (nComp) out.push(chip(`<b>${nComp}</b> component${nComp > 1 ? "s" : ""}`));
    // Display unsupported preview effects so users can see when preview output differs from native
    // rendering.
    const skipped = player.stats?.degradations ?? [];
    if (skipped.length) {
      const kinds = [...new Set(skipped.map((d) => d.effect))].join(" · ");
      out.push(chip(`Preview skipped <b>${skipped.length}</b> effects: ${kinds}`, "warn"));
    }
    $("meta").innerHTML = out.join("");
  }
  function layoutForTimeline(cfg: DemoConfig): void {
    fps = fpsFrom(cfg);
    const dur = Math.max(0.001, player.durationS());
    scrub.max = String(dur);
    scrub.step = String(1 / Math.max(1, fps));
    const cv = cfg.timeline.document.canvas;
    if (cv?.width && cv?.height) {
      stageWrap.style.aspectRatio = `${cv.width} / ${cv.height}`;
      // Expose the numeric aspect ratio for CSS width calculations based on the available height.
      stageWrap.style.setProperty("--stage-ar", String(cv.width / cv.height));
    }
    renderMeta(cfg);
    updateAudioUI();
    syncTransport();
  }

  // ── transport (play state + scrubber + readout, kept in sync) ──
  const PLAY_PATH = "M8 5.5v13l11-6.5z";
  const PAUSE_PATH = "M7 5h3.5v14H7zM13.5 5H17v14h-3.5z";
  let loopEnabled = false;
  let rafPoll: number | null = null;

  function setPlayingUI(playing: boolean): void {
    stageWrap.dataset.playing = playing ? "true" : "false";
    $("playIcon").setAttribute("d", playing ? PAUSE_PATH : PLAY_PATH);
    $("playToggle").setAttribute("aria-label", playing ? "Pause" : "Play");
    $("bigplay").innerHTML = playing
      ? '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M7 5h3.5v14H7zM13.5 5H17v14h-3.5z"/></svg>'
      : '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M8 5.5v13l11-6.5z"/></svg>';
  }
  function syncTransport(): void {
    const t = player.currentTime();
    const dur = player.durationS();
    scrub.value = String(t);
    scrub.style.setProperty("--pct", `${dur > 0 ? (t / dur) * 100 : 0}%`);
    $("tCur").textContent = fmtTime(t);
    $("tDur").textContent = fmtTime(dur);
    $("tFrame").textContent = `f${frameOf(t)}/${frameOf(dur)}`;
    setPlayingUI(player.playing);
  }
  function startPoll(): void {
    if (rafPoll != null) return;
    const step = () => {
      syncTransport();
      if (player.playing) {
        rafPoll = requestAnimationFrame(step);
      } else {
        rafPoll = null;
        // Restart from zero when playback reaches the end with looping enabled.
        if (loopEnabled && player.currentTime() >= player.lastFrameTimeS() - 1e-6) doPlay();
      }
    };
    rafPoll = requestAnimationFrame(step);
  }
  async function doPlay(): Promise<void> { await player.play(); setPlayingUI(true); startPoll(); }
  function doPause(): void { player.pause(); syncTransport(); }
  function togglePlay(): void { player.playing ? doPause() : void doPlay(); }
  async function seekTo(t: number): Promise<void> {
    if (player.playing) player.pause();
    await player.seek(Math.max(0, Math.min(t, player.lastFrameTimeS())));
    syncTransport();
  }
  const stepFrames = (n: number): Promise<void> => seekTo(player.currentTime() + n / Math.max(1, fps));
  const seekBy = (dt: number): Promise<void> => seekTo(player.currentTime() + dt);
  // Range input can fire faster than a full component frame. Keep only the newest target so
  // current-priority Worker jobs and CanvasKit presentation stay single-flight.
  let scrubTargetS: number | null = null, scrubBusy = false;
  function scrubTo(t: number): void {
    scrubTargetS = t;
    if (scrubBusy) return;
    scrubBusy = true;
    void (async () => {
      try {
        while (scrubTargetS != null) {
          const next = scrubTargetS;
          scrubTargetS = null;
          await player.seek(next);
        }
      } catch (error) {
        showError(errorText(error));
      } finally {
        scrubBusy = false;
      }
    })();
  }

  // Update transport after seeks; playback updates remain driven by the animation-frame poll.
  player.addEventListener("time", () => { if (!player.playing) syncTransport(); });

  // ── wire controls ──
  $("hit").addEventListener("click", togglePlay);
  $("playToggle").addEventListener("click", togglePlay);
  $("toStart").addEventListener("click", () => seekTo(0));
  $("toEnd").addEventListener("click", () => seekTo(player.lastFrameTimeS()));
  $("prevFrame").addEventListener("click", () => stepFrames(-1));
  $("nextFrame").addEventListener("click", () => stepFrames(1));
  $("loopBtn").addEventListener("click", () => {
    loopEnabled = !loopEnabled;
    $("loopBtn").setAttribute("aria-pressed", loopEnabled ? "true" : "false");
  });
  $("fsBtn").addEventListener("click", () => {
    if (document.fullscreenElement) document.exitFullscreen();
    else stageWrap.requestFullscreen?.().catch(() => {});
  });
  // Pause before scrubbing to avoid competing with the playback loop.
  scrub.addEventListener("pointerdown", () => { if (player.playing) player.pause(); });
  scrub.addEventListener("input", () => {
    scrubTo(Number(scrub.value));
    scrub.style.setProperty("--pct", `${(Number(scrub.value) / Math.max(1e-6, player.durationS())) * 100}%`);
    $("tCur").textContent = fmtTime(scrub.value);
    $("tFrame").textContent = `f${frameOf(scrub.value)}/${frameOf(player.durationS())}`;
    setPlayingUI(false);
  });
  // Show hover time without changing playback position.
  const seektip = $<HTMLElement>("seektip");
  scrub.addEventListener("pointermove", (e) => {
    const rect = scrub.getBoundingClientRect();
    const frac = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    seektip.textContent = fmtTime(frac * player.durationS());
    seektip.style.left = `${frac * 100}%`;
    seektip.hidden = false;
  });
  scrub.addEventListener("pointerleave", () => { seektip.hidden = true; });

  // Volume controls, shown only when the timeline has audio.
  const volGroup = $<HTMLElement>("volGroup");
  const vol = $<HTMLInputElement>("vol");
  function updateVolIcon(): void {
    const muted = player.muted || player.masterVolume === 0;
    $("muteBtn").setAttribute("aria-pressed", muted ? "true" : "false");
    $("volIcon").innerHTML = muted
      ? '<path d="M4 9v6h4l5 4V5L8 9H4z"/><path d="M22 9l-6 6M16 9l6 6" stroke="currentColor" stroke-width="2" fill="none" stroke-linecap="round"/>'
      : '<path d="M4 9v6h4l5 4V5L8 9H4z"/><path d="M16 8.5a4 4 0 0 1 0 7" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>';
  }
  function updateAudioUI(): void {
    let has = false;
    try { has = player.hasAudio(); } catch { has = false; }
    volGroup.hidden = !has;
    vol.value = String(player.masterVolume);
    vol.style.setProperty("--vpct", `${player.masterVolume * 100}%`);
    updateVolIcon();
  }
  function toggleMute(): void { player.setMuted(!player.muted); updateVolIcon(); }
  vol.addEventListener("input", () => {
    player.setMuted(false);
    const v = player.setVolume(Number(vol.value));
    vol.style.setProperty("--vpct", `${v * 100}%`);
    updateVolIcon();
  });
  $("muteBtn").addEventListener("click", toggleMute);

  // Performance HUD using frame rate, stage timings, surface mode, and audio status.
  const hud = $<HTMLElement>("hud");
  function toggleHud(): void {
    hud.hidden = !hud.hidden;
    $("hudBtn").setAttribute("aria-pressed", hud.hidden ? "false" : "true");
  }
  $("hudBtn").addEventListener("click", toggleHud);
  const per = (ms: number, r: number): number => (r > 0 ? ms / r : 0);
  const hudRow = (k: string, v: string, cls = ""): string => `<div class="row"><span class="k">${k}</span><span class="v ${cls}">${v}</span></div>`;
  // Measure actual renders per second and compare against the content frame rate, independent of
  // display refresh rate.
  let lastRenders = 0, lastT = performance.now(), fpsVal = 0;
  function updateHud(): void {
    if (hud.hidden) return;
    const s = player.stats;
    const now = performance.now();
    const dt = now - lastT;
    if (dt >= 250) { fpsVal = ((s.renders - lastRenders) * 1000) / dt; lastRenders = s.renders; lastT = now; }
    const r = Math.max(1, s.renders);
    const frameMs = per(s.perfFrameMs, r);
    // Show a neutral dash while paused; frame-rate thresholds apply only during playback.
    const fpsText = player.playing ? fpsVal.toFixed(0) : "—";
    const fpsCls = !player.playing ? "" : fpsVal >= fps * 0.95 ? "good" : fpsVal >= fps * 0.6 ? "" : "warn";
    hud.innerHTML =
      hudRow("fps", `<span class="fps">${fpsText}</span>`, fpsCls) +
      hudRow("frame", `${frameMs.toFixed(1)} ms`) + "<hr>" +
      hudRow("evaluate / lower", `${per(s.perfEvaluatePrepareMs, r).toFixed(2)} / ${per(s.perfLowerMs, r).toFixed(2)} ms`) +
      hudRow("fulfill (overlap)", `${per(s.perfResourceFulfillMs, r).toFixed(2)} ms`) +
      hudRow("bind / execute", `${per(s.perfBindPacketsMs, r).toFixed(2)} / ${per(s.perfExecuteMs, r).toFixed(2)} ms`) +
      hudRow("product packets", `${(per(s.planBytes, r) / 1024).toFixed(1)} / ${(per(s.bindingBytes, r) / 1024).toFixed(1)} KiB`) +
      hudRow("product graph", `${per(s.executionPasses, r).toFixed(1)} passes · ${s.physicalSurfaces} surfaces · ${s.maximumLiveImages} live`) +
      hudRow("working surfaces", `${s.surfaceAllocations} alloc · ${s.surfaceReuses} reuse · ${(s.maximumSurfaceResidentBytes / 1048576).toFixed(1)} MiB`) +
      hudRow("plan cache", `${s.templateCacheHits} hit · ${s.templateCacheMisses} miss`) +
      hudRow("program/font/shader cache", `${s.programCacheHits}/${s.fontCacheHits}/${s.shaderCacheHits} hit`) +
      hudRow("prepared", `${per(s.preparedFrames, r).toFixed(1)} frames/render`) +
      hudRow("· video decode/wrap", `${per(s.perfVideoDecodeMs, r).toFixed(2)} / ${(per(s.perfVideoCopyMs, r) + per(s.perfVideoWrapMs, r)).toFixed(2)} ms`) +
      hudRow("present / capture", `${per(s.perfPresentMs, r).toFixed(2)} / ${per(s.perfCaptureEncodeMs, r).toFixed(2)} ms`) + "<hr>" +
      hudRow("surface", (s.surfaceMode || "cpu").toUpperCase(), s.surfaceMode.startsWith("gpu") ? "good" : "") +
      hudRow("audio", s.audioMode || "none") +
      hudRow("renders", String(s.renders));
  }
  setInterval(updateHud, 200);

  window.addEventListener("keydown", (e) => {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    let handled = true;
    switch (e.key) {
      case " ": case "k": togglePlay(); break;
      case "ArrowLeft": case ",": stepFrames(e.shiftKey ? -Math.round(fps) : -1); break;
      case "ArrowRight": case ".": stepFrames(e.shiftKey ? Math.round(fps) : 1); break;
      case "j": seekBy(-1); break;
      case "l": seekBy(1); break;
      case "Home": seekTo(0); break;
      case "End": seekTo(player.lastFrameTimeS()); break;
      case "f": case "F": $("fsBtn").click(); break;
      case "m": case "M": toggleMute(); break;
      case "i": case "I": toggleHud(); break;
      default: handled = false;
    }
    if (handled) e.preventDefault();
  });

  layoutForTimeline(config);

  // Reload configuration on timeline SSE events and replace the timeline in place. Static smoke
  // hosts may have no event endpoint.
  const observeMode = params.get("observe") === "1";
  let generation = config.generation ?? 1;
  async function postObservation(status: string): Promise<void> {
    const rendered = await player.scrub(player.currentTime());
    await fetch("/result", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        status,
        generation,
        passes: rendered.stats.passes,
        programs: rendered.stats.programs,
        stats: player.stats,
        pngBase64: bytesToBase64(requiredPng(rendered.png)),
      }),
    });
  }
  if (typeof EventSource === "function") {
    const events = new EventSource("/events");
    events.addEventListener("timeline", async () => {
      try {
        const next = await fetchConfig();
        if (next.generation != null && next.generation === generation) return;
        generation = next.generation ?? generation + 1;
        await player.replaceRenderPackage(next);
        layoutForTimeline(next);
        if (observeMode) await postObservation("hot-reload-applied");
      } catch (err) {
        showError(errorText(err));
        await postFailure(errorText(err));
      }
    });
    // Reload the page when runtime source changes; replacing timeline data cannot update loaded
    // runtime code.
    events.addEventListener("runtime", () => location.reload());
  }

  function requiredPng(bytes: Uint8Array | null): Uint8Array {
    if (!bytes) throw new Error("player capture did not return PNG bytes");
    return bytes;
  }

  function bytesToBase64(bytes: Uint8Array): string {
    let bin = "";
    const chunk = 0x8000;
    for (let i = 0; i < bytes.length; i += chunk) {
      bin += String.fromCharCode(...bytes.subarray(i, i + chunk));
    }
    return btoa(bin);
  }

  async function runSmoke(): Promise<void> {
    const captures: Array<Record<string, unknown>> = [];
    let stage = "initial-seek";
    try {
    if (config.initialTimeS != null) {
      await player.seek(config.initialTimeS);
    }
    // Keep playback-window deltas alongside lifetime counters. Component runtime/CanvasKit cold
    // start happens before this point and otherwise obscures whether misses occur during play.
    const smokeBaseline = { ...player.stats };
    const playT0 = performance.now();
    stage = "play";
    await player.play();
    // Allow a longer playback window for frame-rate measurements; the default only exercises
    // startup.
    const playMs = Number(params.get("playMs")) || config.playMs || 180;
    await new Promise<void>((resolve) => setTimeout(resolve, playMs));
    stage = "pause";
    player.pause();
    // Measure elapsed playback time because busy test hosts may delay timers.
    const smokePlayedMs = performance.now() - playT0;
    // Read the visible canvas to verify nonblack presentation for CPU and GPU paths. CPU captures
    // alone cannot verify direct GPU presentation.
    let presentNonBlackPixels = 0;
    stage = "present-readback";
    {
      const canvas = player.canvas;
      if (!canvas) throw new Error("<valle-player> does not own a canvas");
      const context = canvas.getContext("2d");
      if (!context) throw new Error("demo canvas does not expose a 2D context");
      const d = context.getImageData(0, 0, canvas.width, canvas.height).data;
      for (let i = 0; i < d.length; i += 4) {
        if (d[i] || d[i + 1] || d[i + 2]) presentNonBlackPixels += 1;
      }
    }
    stage = "captures";
    for (const capture of config.captures ?? []) {
      const rendered = await player.scrub(capture.timeS);
      scrub.value = String(capture.timeS);
      captures.push({
        sampleId: capture.sampleId,
        timeS: capture.timeS,
        implementation: rendered.implementation,
        runtimeFlavor: rendered.runtimeFlavor,
        executionProfile: rendered.executionProfile,
        fontsRegistered: rendered.fontsRegistered,
        passes: rendered.stats.passes,
        programs: rendered.stats.programs,
        videoFrames: rendered.stats.videoFrames,
        lottieFrames: rendered.stats.lottieFrames,
        pngBase64: bytesToBase64(requiredPng(rendered.png)),
      });
    }
    stage = "report";
    await fetch("/result", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        status: "ok",
        implementation: captures[0]?.implementation ?? "valle-web-player-browser",
        userAgent: navigator.userAgent,
        stats: {
          ...player.stats,
          smokePlayedMs,
          smokeRenders: player.stats.renders - smokeBaseline.renders,
          smokeFrameMs: player.stats.perfFrameMs - smokeBaseline.perfFrameMs,
          smokePreparedFrames: player.stats.preparedFrames - smokeBaseline.preparedFrames,
          smokeSurfaceAllocations: player.stats.surfaceAllocations - smokeBaseline.surfaceAllocations,
          smokeSurfaceReuses: player.stats.surfaceReuses - smokeBaseline.surfaceReuses,
          presentNonBlackPixels,
        },
        captures,
      }),
    });
    } catch (error) {
      throw new Error(scopedError(`smoke.${stage}`, error));
    }
  }

  // Render offline audio and report RMS in 100 ms windows to verify gain, fades, looping, and
  // mixing without user gestures.
  async function runAudioProbe(): Promise<void> {
    const t1 = Number(params.get("audioT1")) || player.durationS();
    const rendered = await player.renderAudioOffline(0, t1, { sampleRate: 24000, channels: 1 });
    if (!rendered) throw new Error("audio probe timeline has no schedulable audio");
    const data = rendered.getChannelData(0);
    const win = Math.round(rendered.sampleRate * 0.1);
    const rms = [];
    for (let i = 0; i + win <= data.length; i += win) {
      let acc = 0;
      for (let j = i; j < i + win; j += 1) acc += data[j] * data[j];
      rms.push(Number(Math.sqrt(acc / win).toFixed(5)));
    }
    await fetch("/result", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        status: "audio-probe",
        generation,
        audio: {
          sampleRate: rendered.sampleRate,
          windowMs: 100,
          scheduledSources: player.stats.audioScheduledSources,
          decodeErrors: player.stats.audioDecodeErrors,
          rms,
        },
      }),
    });
  }

  // Explicit browser lifecycle probe for content replacement, readiness, playback, detachment,
  // reattachment, and release of the previous runtime.
  async function runComponentLifecycleProbe(): Promise<void> {
    const emptyResult = await player.replaceRenderPackage(config);
    const emptyCapture = emptyResult ?? await player.captureFrame(0);
    const contentResult = await player.replaceRenderPackage(config);
    const contentCapture = contentResult ?? await player.captureFrame(0);

    const oldController = player.controller;
    if (!oldController) throw new Error("component probe requires a ready controller");
    player.remove();
    await oldController.dispose();
    stageWrap.prepend(player);
    await player.load();
    const newController = player.controller;
    if (!newController || newController === oldController) {
      throw new Error("reattach must create a fresh controller");
    }

    let timeEvents = 0;
    const onTime = () => { timeEvents += 1; };
    player.addEventListener("time", onTime);
    await player.play();
    await new Promise<void>((resolve) => setTimeout(resolve, 120));
    player.pause();
    player.removeEventListener("time", onTime);
    if (timeEvents === 0) throw new Error("reattached player did not advance its clock");

    const lifecycle = {
      shadowCanvas: player.shadowRoot?.querySelectorAll("canvas").length ?? 0,
      shadowLoading: player.shadowRoot?.querySelectorAll('[part="loading"]').length ?? 0,
      shadowError: player.shadowRoot?.querySelectorAll('[part="error"]').length ?? 0,
      shadowSlot: player.shadowRoot?.querySelectorAll("slot").length ?? 0,
      emptyFlavor: emptyCapture.runtimeFlavor,
      contentFlavor: contentCapture.runtimeFlavor,
      oldState: oldController.state,
      newState: newController.state,
      timeEvents,
    };
    if (
      lifecycle.shadowCanvas !== 1
      || lifecycle.shadowLoading !== 1
      || lifecycle.shadowError !== 1
      || lifecycle.shadowSlot !== 1
      || lifecycle.emptyFlavor !== lifecycle.contentFlavor
      || (lifecycle.contentFlavor !== "product-gpu" && lifecycle.contentFlavor !== "product-cpu")
      || lifecycle.oldState !== "disposed"
      || lifecycle.newState !== "ready"
    ) {
      throw new Error(`component lifecycle contract failed: ${JSON.stringify(lifecycle)}`);
    }
    stageWrap.dataset.componentProbe = "ok";
    $("meta").insertAdjacentHTML("beforeend", chip("<b>component lifecycle</b> ok", "live"));
    await fetch("/result", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        status: "ok",
        implementation: "valle-player-lit@1",
        userAgent: navigator.userAgent,
        stats: { componentLifecycle: lifecycle },
        captures: [],
      }),
    });
  }

  if (params.get("componentprobe") === "1") {
    await runComponentLifecycleProbe().catch((err) => postFailure(errorText(err)));
  } else if (params.get("smoke") === "1") {
    runSmoke().catch(async (err) => {
      await fetch("/result", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ status: "error", message: errorText(err) }),
      });
    });
  } else if (params.get("audioprobe") === "1") {
    await runAudioProbe().catch((err) => postFailure(errorText(err)));
  } else if (observeMode) {
    // Capture the initial frame and each applied update for automated observation.
    await player.seek(config.initialTimeS ?? 0);
    syncTransport();
    await postObservation("observe-ready");
  } else {
    await player.seek(0);
    syncTransport();
  }
