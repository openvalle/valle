import { loadStudioHost, type StudioHost } from "./host.ts";
import type { ValleStudioApp } from "./studio-shell.ts";
import { startTimelineStudio } from "./timeline-workspace.ts";
import { StudioSourceEditor } from "./source-editor.ts";
import { StandaloneTimelineSession } from "./standalone-timeline.ts";
import { TimelineBrowserSession } from "./timeline-browser-session.ts";
import { hydrateIcons } from "../../shared/icons.ts";
import "./studio-shell.ts";
import "./timeline-components.ts";
import "valle-engine/element";

export { initialStudioShellState } from "./shell-state.ts";

declare global {
  var valleStudioHost: StudioHost | undefined;
}

hydrateIcons();

const shell = document.querySelector<ValleStudioApp>("valle-studio-app");
if (!shell) throw new Error("missing <valle-studio-app> root");

const host = await loadStudioHost(location.search);
globalThis.valleStudioHost = host;
await shell.initialize(host);
const sourceEditor = await StudioSourceEditor.mount(host);
let timelineSessionStarted = false;
const syncSourceDirty = () => shell.dispatchIntent({ type: "source-dirty", value: sourceEditor.dirty });
for (const event of ["edited", "saved", "reloaded", "reconciled", "restored"]) {
  sourceEditor.addEventListener(event, syncSourceDirty);
}
syncSourceDirty();
shell.addEventListener("studio-host-event", (event) => {
  const type = (event as CustomEvent<{ type: string }>).detail.type;
  if (type === "motion" || type === "timeline" || type === "project") {
    void sourceEditor.refreshFromDisk();
  }
});
sourceEditor.addEventListener("save-request", () => {
  if (!timelineSessionStarted && host.boot.session.kind === "motion-file") {
    void sourceEditor.saveDirty();
    return;
  }
  document.getElementById("projectControls")
    ?.dispatchEvent(new CustomEvent("studio-project-intent", {
      detail: { type: "save" }, bubbles: true, composed: true,
    }));
});
window.addEventListener("beforeunload", (event) => {
  if (sourceEditor.dirty) { event.preventDefault(); event.returnValue = ""; }
});

window.addEventListener("keydown", (event) => {
  if (event.isComposing) return;
  const editing = event.composedPath().some((node) => (
    node instanceof HTMLElement
    && (node.matches("input, textarea, select") || node.isContentEditable)
  ));
  if (event.key === "Escape") {
    document.getElementById("dragTip")?.setAttribute("hidden", "");
    shell.classList.remove("mobile-inspector");
    return;
  }
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
    event.preventDefault();
    if (!timelineSessionStarted && host.boot.session.kind === "motion-file") {
      void sourceEditor.saveDirty();
      return;
    }
    document.getElementById("projectControls")
      ?.dispatchEvent(new CustomEvent("studio-project-intent", {
        detail: { type: "save" },
        bubbles: true,
        composed: true,
      }));
    return;
  }
  if (editing || event.isComposing) return;
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "z") {
    event.preventDefault();
    shell.dispatchEvent(new CustomEvent("studio-history-intent", {
      detail: { type: event.shiftKey ? "redo" : "undo" },
      bubbles: true,
      composed: true,
    }));
  }
});

if (host.boot.session.kind === "motion-file") {
  const standalone = new StandaloneTimelineSession(host.boot, sourceEditor, host);
  const saveAsButton = document.getElementById("saveAsTimeline") as HTMLButtonElement;
  saveAsButton.hidden = false;
  saveAsButton.disabled = true;
  saveAsButton.title = standalone.conversionBlockedReason ?? "Save the current arrangement as a Timeline file";
  saveAsButton.addEventListener("click", () => {
    shell.dispatchEvent(new CustomEvent("studio-save-as-timeline", { bubbles: true, composed: true }));
  });
  shell.addEventListener("studio-host-event", (event) => {
    if ((event as CustomEvent<{ type: string }>).detail.type === "motion") {
      standalone.invalidateAssetInputs();
    }
  });
  const start = async (initial: Awaited<ReturnType<typeof standalone.loadTimeline>>) => {
    let first = true;
    await startTimelineStudio({
      sourceEditor,
      loadTimeline: () => {
        if (first) { first = false; return Promise.resolve(initial); }
        return standalone.loadTimeline();
      },
      preparePreview: (timeline) => standalone.prepareTimelinePreview(timeline),
      saveTimeline: (edit) => standalone.saveTimeline(edit),
      prepareTimelineSave: () => standalone.chooseTarget(),
      beforeReload: () => standalone.reloadSavedTimeline(),
      motionInstance: (path) => standalone.motionInstance(path),
    });
    timelineSessionStarted = true;
    saveAsButton.disabled = standalone.conversionBlockedReason !== null;
  };
  window.addEventListener("pagehide", (event) => { if (!event.persisted) standalone.close(); });
  try {
    await start(await standalone.loadTimeline());
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    shell.dispatchIntent({ type: "preview-status", status: "error", message });
    document.getElementById("errtext")!.textContent = message;
    document.getElementById("errbox")!.hidden = false;
    document.getElementById("loading")!.hidden = true;
    sourceEditor.open(sourceEditor.resolvePath(host.boot.session.input));
    let version = 0;
    let started = false;
    const retry = async () => {
      const mine = ++version;
      try {
        const initial = await standalone.loadTimeline();
        if (mine !== version || started) return;
        started = true;
        sourceEditor.removeEventListener("edited", retry);
        document.getElementById("errbox")!.hidden = true;
        await start(initial);
      } catch (failure) {
        if (mine !== version || started) return;
        const diagnostic = failure instanceof Error ? failure.message : String(failure);
        shell.dispatchIntent({ type: "preview-status", status: "error", message: diagnostic });
        document.getElementById("errtext")!.textContent = diagnostic;
      }
    };
    sourceEditor.addEventListener("edited", retry);
  }
} else {
  const browserSession = new TimelineBrowserSession(host, sourceEditor);
  shell.addEventListener("studio-host-event", (event) => {
    const type = (event as CustomEvent<{ type: string }>).detail.type;
    if (type === "timeline" || type === "project") browserSession.invalidateMedia();
  });
  window.addEventListener("pagehide", (event) => { if (!event.persisted) browserSession.close(); });
  await startTimelineStudio({
    sourceEditor,
    preparePreview: (timeline) => browserSession.prepareTimelinePreview(timeline),
  });
  timelineSessionStarted = true;
}
