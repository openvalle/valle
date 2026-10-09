import type { BrowserValleWebPlayerOptions } from "valle-engine";
import type { BrowserVideoExportProgress, BrowserVideoExportSettings } from "valle-engine/export";
import { iconSvg } from "../../shared/icons.ts";
import type { ValleStudioApp } from "./studio-shell.ts";

interface SaveFilePickerWindow extends Window {
  showSaveFilePicker?: (options: {
    suggestedName: string;
    types: Array<{ description: string; accept: Record<string, string[]> }>;
  }) => Promise<FileSystemFileHandle>;
}

interface StudioExportSnapshot {
  input: BrowserValleWebPlayerOptions;
  width: number;
  height: number;
  frameRate: string;
  durationS: number;
  sampleRate: number;
  hasAudio: boolean;
}

type ExportState = "setup" | "running" | "complete" | "cancelled" | "error";

function clock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(total / 60), rest = String(total % 60).padStart(2, "0");
  return minutes >= 60 ? `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, "0")}:${rest}` : `${minutes}:${rest}`;
}

function fileStem(value: string): string {
  return value.trim().replace(/\.mp4$/i, "").replace(/[\\/:*?"<>|\u0000-\u001f]/g, "-")
    .replace(/\.+$/, "").slice(0, 180).trim() || "video";
}

function resolution(source: StudioExportSnapshot, preset: string): { width: number; height: number } {
  if (preset === "source") return { width: source.width, height: source.height };
  const scale = Number(preset) / Math.min(source.width, source.height);
  const even = (value: number) => Math.max(2, Math.round(value / 2) * 2);
  return { width: even(source.width * scale), height: even(source.height * scale) };
}

/** Review and export an admitted preview snapshot without a host preparation request. */
export function bindStudioVideoExport(shell: ValleStudioApp, snapshot: () => StudioExportSnapshot): void {
  const element = <T extends HTMLElement>(id: string): T => {
    const found = document.getElementById(id);
    if (!found) throw new Error(`missing #${id}`);
    return found as T;
  };
  const button = element<HTMLButtonElement>("exportVideo");
  const dialog = element<HTMLDialogElement>("exportVideoDialog");
  const form = element<HTMLFormElement>("exportVideoForm");
  const title = element("exportVideoTitle");
  const status = element("exportVideoStatus");
  const stateIcon = element("exportVideoIcon");
  const name = element<HTMLInputElement>("exportVideoName");
  const fileName = element("exportVideoFileName");
  const hint = element("exportVideoNameHint");
  const settingsFields = element<HTMLFieldSetElement>("exportVideoSettings");
  const resolutionPreset = element<HTMLSelectElement>("exportVideoResolutionPreset");
  const fps = element<HTMLSelectElement>("exportVideoFps");
  const bitrate = element<HTMLInputElement>("exportVideoBitrate");
  const includeAudio = element<HTMLInputElement>("exportVideoIncludeAudio");
  const audioBitrate = element<HTMLSelectElement>("exportVideoAudioBitrate");
  const progress = element<HTMLProgressElement>("exportVideoProgress");
  const phase = element("exportVideoPhase");
  const percent = element("exportVideoPercent");
  const frames = element("exportVideoFrames");
  const note = element("exportVideoNote");
  const details = element<HTMLDetailsElement>("exportVideoErrorDetails");
  const cancel = element<HTMLButtonElement>("exportVideoCancel");
  const close = element<HTMLButtonElement>("exportVideoClose");
  const dismiss = element<HTMLButtonElement>("exportVideoDismiss");
  const start = element<HTMLButtonElement>("exportVideoStart");
  const picker = (window as SaveFilePickerWindow).showSaveFilePicker;
  let prepared: StudioExportSnapshot | null = null;
  let active: AbortController | null = null;
  let startedAt: number | null = null;
  let timer: ReturnType<typeof setInterval> | undefined;
  let state: ExportState = "setup";
  let manualBitrate = false;

  const sync = () => {
    button.disabled = active !== null || shell.shellState.previewStatus !== "ready";
    button.title = active ? "Export in progress" : shell.shellState.previewStatus === "ready"
      ? "Export video" : "Export is available when the preview is up to date";
  };
  const elapsed = () => clock(startedAt === null ? 0 : (performance.now() - startedAt) / 1000);
  const finishedIn = () => {
    const seconds = startedAt === null ? 0 : (performance.now() - startedAt) / 1000;
    return seconds < 60 ? `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)}s` : clock(seconds);
  };
  const sourceFps = () => {
    const [numerator, denominator] = prepared!.frameRate.split("/").map(Number);
    return numerator! / denominator!;
  };
  const selectedSettings = (): BrowserVideoExportSettings => ({
    ...resolution(prepared!, resolutionPreset.value),
    ...(fps.value === "source" ? {} : { frameRate: Number(fps.value) }),
    videoBitrate: Math.round(bitrate.valueAsNumber * 1_000_000),
    includeAudio: prepared!.hasAudio && includeAudio.checked,
    audioBitrate: Number(audioBitrate.value),
  });
  const syncSettings = () => {
    if (!prepared) return;
    const size = resolution(prepared, resolutionPreset.value);
    const rate = fps.value === "source" ? sourceFps() : Number(fps.value);
    // A practical starting point for H.264; the user can always choose another target.
    const suggested = Math.min(200, Math.max(1, Math.round(size.width * size.height * rate * 0.12 / 100_000) / 10));
    if (!manualBitrate) bitrate.value = String(suggested);
    element("exportVideoBitrateHint").textContent = `Suggested: ${suggested} Mbps`;
    audioBitrate.disabled = !prepared.hasAudio || !includeAudio.checked;
    element("exportVideoAudioSettings").dataset.disabled = String(audioBitrate.disabled);
    const settings = selectedSettings();
    const estimatedBytes = ((settings.videoBitrate ?? 0) + (settings.includeAudio ? settings.audioBitrate! : 0)) * prepared.durationS / 8;
    note.textContent = Number.isFinite(estimatedBytes) && estimatedBytes > 0
      ? `Estimated ${estimatedBytes < 1_000_000 ? `${Math.round(estimatedBytes / 1000)} KB` : `${(estimatedBytes / 1_000_000).toFixed(1)} MB`}` : "Estimated size —";
  };
  const setState = (next: ExportState, message?: string) => {
    state = next;
    dialog.dataset.state = next;
    dialog.setAttribute("aria-busy", String(next === "running"));
    element("exportVideoNameField").hidden = next !== "setup";
    name.disabled = next !== "setup";
    fileName.hidden = next === "setup";
    settingsFields.hidden = next !== "setup";
    settingsFields.disabled = next !== "setup";
    element("exportVideoFacts").hidden = next === "setup";
    hint.hidden = next !== "setup";
    element("exportVideoProgressGroup").hidden = next !== "running";
    details.hidden = next !== "error";
    dismiss.hidden = next === "running";
    cancel.hidden = next !== "setup" && next !== "running";
    cancel.disabled = false;
    cancel.textContent = next === "running" ? "Cancel export" : "Cancel";
    close.hidden = next === "setup" || next === "running";
    close.textContent = next === "complete" ? "Done" : "Close";
    start.hidden = next === "running" || next === "complete";
    element("exportVideoStartLabel").textContent = next === "error" ? "Try again" : next === "cancelled" ? "Back to export" : "Export video";
    element("exportVideoNameLabel").textContent = next === "setup" ? "File name" : "MP4 video";
    stateIcon.innerHTML = iconSvg(next === "complete" ? "check" : next === "error" ? "alert" : next === "cancelled" ? "close" : "download");
    const copy: Record<ExportState, [string, string]> = {
      setup: ["Export video", "Choose your settings and save an MP4."],
      running: ["Exporting video", "Keep this tab open until the export finishes."],
      complete: [picker ? "Video exported" : "Download started", picker
        ? "Your video is saved and ready to share." : "Find your MP4 in your browser’s downloads."],
      cancelled: ["Export cancelled", "Your export was stopped. You can start again whenever you’re ready."],
      error: ["Couldn’t export video", "The export didn’t finish. Try again, or check the details below."],
    };
    title.textContent = copy[next][0];
    status.textContent = message ?? copy[next][1];
    if (next === "setup") syncSettings();
    else note.textContent = next === "complete" ? `Finished in ${finishedIn()}`
      : next === "cancelled" || next === "error" ? "Not saved" : "Waiting to start…";
  };
  const showError = (error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    element("exportVideoError").textContent = error instanceof Error ? error.stack ?? message : message;
    let explanation: string | undefined;
    if (message.includes("even canvas")) explanation = "MP4 needs an even width and height. Choose a resolution preset and try again.";
    else if (/cannot encode|cannot export MP4|cannot verify AAC|AAC encoding support/.test(message)) explanation = message;
    setState("error", explanation);
  };
  const prepare = (reset: boolean) => {
    details.open = false;
    startedAt = null;
    try {
      prepared = snapshot();
      if (reset) {
        name.value = fileStem(shell.shellState.projectName.replace(/\.(?:(?:motion|timeline)\.)?(?:tsx|jsx|json)$/i, ""));
        resolutionPreset.value = "source";
        fps.value = "source";
        includeAudio.checked = prepared.hasAudio;
        audioBitrate.value = "192000";
        manualBitrate = false;
      }
      name.setCustomValidity("");
      for (const option of resolutionPreset.options) {
        const size = resolution(prepared, option.value);
        const label = option.value === "source" ? "Original" : option.value === "2160" ? "4K" : `${option.value}p`;
        option.textContent = `${label} · ${size.width} × ${size.height}`;
        option.disabled = size.width > 8192 || size.height > 8192;
      }
      fps.options[0]!.textContent = `Original · ${Number(sourceFps().toFixed(3))} fps`;
      includeAudio.disabled = !prepared.hasAudio;
      element("exportVideoAudioFormat").textContent = prepared.hasAudio ? `Stereo · ${prepared.sampleRate / 1000} kHz` : "No audio in this timeline";
      element("exportVideoSourceSummary").textContent = `${clock(prepared.durationS)} · Full timeline`;
      hint.textContent = picker ? "Choose where to save in the next step." : "Your MP4 will be saved to your browser’s downloads.";
      setState("setup");
    } catch (error) {
      prepared = null;
      showError(error);
    }
    if (!dialog.open) dialog.showModal();
    (state === "setup" ? start : close).focus({ preventScroll: true });
  };
  const update = (value: BrowserVideoExportProgress) => {
    if (active?.signal.aborted) return;
    if (value.phase === "loading") {
      progress.removeAttribute("value");
      phase.textContent = "Preparing video";
      percent.textContent = "Starting…";
      frames.textContent = "Loading your timeline and media…";
    } else {
      // Final file commit, rather than the last rendered frame, completes the export.
      progress.max = 100;
      progress.value = value.phase === "finalizing" ? 99 : value.framesCompleted / Math.max(1, value.frameCount) * 99;
      percent.textContent = `${Math.floor(progress.value)}%`;
      phase.textContent = value.phase === "finalizing" ? "Finishing file" : "Rendering video";
      frames.textContent = value.phase === "finalizing" ? "Saving the finished MP4…"
        : `${value.framesCompleted.toLocaleString("en")} of ${value.frameCount.toLocaleString("en")} frames`;
    }
  };
  const abort = () => {
    if (!active || active.signal.aborted) return;
    active.abort();
    cancel.disabled = true;
    cancel.textContent = "Cancelling…";
    phase.textContent = "Cancelling export";
    frames.textContent = "Stopping and cleaning up…";
  };

  shell.addEventListener("studio-statechange", sync);
  sync();
  name.addEventListener("input", () => name.setCustomValidity(name.value.trim() ? "" : "Enter a file name."));
  bitrate.addEventListener("input", () => { manualBitrate = true; syncSettings(); });
  for (const control of [resolutionPreset, fps, includeAudio, audioBitrate]) control.addEventListener("change", syncSettings);
  cancel.addEventListener("click", () => active ? abort() : dialog.close());
  close.addEventListener("click", () => dialog.close());
  dismiss.addEventListener("click", () => dialog.close());
  dialog.addEventListener("close", () => {
    button.setAttribute("aria-expanded", "false");
    button.focus({ preventScroll: true });
  });
  dialog.addEventListener("cancel", (event) => {
    if (active) { event.preventDefault(); abort(); }
  });
  // Modal keyboard input must not seek, play, save or undo the timeline behind it.
  dialog.addEventListener("keydown", (event) => {
    event.stopPropagation();
    if ((event.metaKey || event.ctrlKey) && ["s", "z"].includes(event.key.toLowerCase())) event.preventDefault();
  });
  window.addEventListener("pagehide", (event) => {
    if (!event.persisted) { active?.abort(); clearInterval(timer); }
  });
  button.addEventListener("click", () => {
    if (active || button.disabled) return;
    prepare(true);
    button.setAttribute("aria-expanded", "true");
  });

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    if (active) return;
    if (state !== "setup") { prepare(false); return; }
    if (!prepared) return;
    name.setCustomValidity(name.value.trim() ? "" : "Enter a file name.");
    if (!form.reportValidity()) return;
    const input = prepared.input;
    const settings = selectedSettings();
    name.value = fileStem(name.value);
    const suggestedName = `${name.value}.mp4`;
    fileName.textContent = suggestedName;
    element("exportVideoResolution").textContent = `${settings.width} × ${settings.height}`;
    element("exportVideoFrameRate").textContent = `${Number((settings.frameRate ?? sourceFps()).toFixed(3))} fps`;
    element("exportVideoDuration").textContent = clock(prepared.durationS);
    element("exportVideoAudio").textContent = settings.includeAudio ? "Stereo" : "No audio";
    element("exportVideoVideoBitrate").textContent = `${settings.videoBitrate! / 1_000_000} Mbps`;
    element("exportVideoAudioBitrateValue").textContent = settings.includeAudio ? `${settings.audioBitrate! / 1000} kbps` : "—";
    const controller = new AbortController();
    active = controller;
    sync();
    setState("running");
    progress.removeAttribute("value");
    phase.textContent = picker ? "Choose a save location" : "Preparing video";
    percent.textContent = "Starting…";
    frames.textContent = picker ? "Choose a folder for your MP4." : "Loading your timeline and media…";
    cancel.focus({ preventScroll: true });
    void (async () => {
      let writable: FileSystemWritableFileStream | undefined;
      let exporterOwnsWritable = false;
      let choosingLocation = false;
      try {
        if (picker) {
          choosingLocation = true;
          const handle = await picker.call(window, {
            suggestedName,
            types: [{ description: "MP4 video", accept: { "video/mp4": [".mp4"] } }],
          });
          choosingLocation = false;
          controller.signal.throwIfAborted();
          fileName.textContent = handle.name;
          writable = await handle.createWritable();
        }
        controller.signal.throwIfAborted();
        startedAt = performance.now();
        const tick = () => { note.textContent = `${elapsed()} elapsed`; };
        tick();
        timer = setInterval(tick, 1000);
        const { exportBrowserVideo } = await import("valle-engine/export");
        exporterOwnsWritable = true;
        const result = await exportBrowserVideo(input, { ...settings, writable, signal: controller.signal, onProgress: update });
        if (result.blob) {
          const url = URL.createObjectURL(result.blob);
          const link = document.createElement("a");
          link.href = url;
          link.download = suggestedName;
          link.click();
          setTimeout(() => URL.revokeObjectURL(url), 60_000);
        }
        setState("complete");
        close.focus({ preventScroll: true });
      } catch (error) {
        if (!exporterOwnsWritable) await writable?.abort(error).catch(() => undefined);
        const isAbort = error instanceof DOMException && error.name === "AbortError";
        if (choosingLocation && isAbort && !controller.signal.aborted) {
          // Cancelling the OS picker returns to the settings, preserving all choices.
          setState("setup");
          start.focus({ preventScroll: true });
        } else if (controller.signal.aborted || isAbort) {
          setState("cancelled");
          start.focus({ preventScroll: true });
        } else {
          showError(error);
          start.focus({ preventScroll: true });
        }
      } finally {
        clearInterval(timer);
        active = null;
        sync();
      }
    })();
  });
}
