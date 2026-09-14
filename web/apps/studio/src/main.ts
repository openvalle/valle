import { loadStudioHost, type StudioHost } from "./host.ts";
import type { ValleStudioApp } from "./studio-shell.ts";
import { startTimelineStudio } from "./timeline-workspace.ts";
import { startMotionStudio } from "./motion-workspace.ts";
import { hydrateIcons } from "../../shared/icons.ts";
import "./studio-shell.ts";
import "./timeline-components.ts";
import "./motion-components.ts";
import "@valle/player";

export type { StudioWorkspace } from "./shell-state.ts";
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
  await startMotionStudio(shell, host);
} else {
  await startTimelineStudio({
    openMotion: (session) => startMotionStudio(shell, host, session),
  });
}
