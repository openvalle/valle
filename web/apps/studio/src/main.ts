import { loadStudioHost, type StudioHost } from "./host.ts";
import type { ValleStudioApp } from "./studio-shell.ts";
import { startTimelineStudio } from "./timeline-workspace.ts";
import { startMotionStudio } from "./motion-workspace.ts";
import "./studio-shell.ts";
import "./timeline-components.ts";
import "./motion-components.ts";
import "@valle/player";

export type { StudioWorkspace } from "./shell-state.ts";
export { initialStudioShellState } from "./shell-state.ts";

declare global {
  var valleStudioHost: StudioHost | undefined;
}

const shell = document.querySelector<ValleStudioApp>("valle-studio-app");
if (!shell) throw new Error("missing <valle-studio-app> root");

const host = await loadStudioHost(location.search);
globalThis.valleStudioHost = host;
await shell.initialize(host);

if (host.boot.session.kind === "motion-file") {
  await startMotionStudio(shell, host);
} else {
  await startTimelineStudio({
    openMotion: (session) => startMotionStudio(shell, host, session),
  });
}
