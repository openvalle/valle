import type { StudioSession } from "./host.ts";

export type StudioWorkspace =
  | { kind: "timeline" }
  | { kind: "motion"; clipId?: string; source: string };

export interface StudioReturnPoint {
  timeS: number;
  playing: boolean;
  selectedClipId: string | null;
  scrollLeft: number;
  zoom: number;
  dirty: boolean;
}

export interface StudioShellState {
  session: StudioSession | null;
  workspace: StudioWorkspace;
  selectedClipId: string | null;
  dirty: boolean;
  conflict: boolean;
  diagnostics: ReadonlyArray<string>;
  returnPoint: StudioReturnPoint | null;
}

export type StudioIntent =
  | { type: "select"; clipId: string | null }
  | { type: "dirty"; value: boolean }
  | { type: "conflict"; value: boolean }
  | { type: "workspace"; workspace: StudioWorkspace; returnPoint?: StudioReturnPoint }
  | { type: "diagnostics"; messages: ReadonlyArray<string> };

export const initialStudioShellState: StudioShellState = {
  session: null,
  workspace: { kind: "timeline" },
  selectedClipId: null,
  dirty: false,
  conflict: false,
  diagnostics: [],
  returnPoint: null,
};

export function reduceStudioIntent(
  state: StudioShellState,
  intent: StudioIntent,
): StudioShellState {
  switch (intent.type) {
    case "select":
      return { ...state, selectedClipId: intent.clipId };
    case "dirty":
      return { ...state, dirty: intent.value };
    case "conflict":
      return { ...state, conflict: intent.value };
    case "diagnostics":
      return { ...state, diagnostics: [...intent.messages] };
    case "workspace":
      return {
        ...state,
        workspace: intent.workspace,
        returnPoint: intent.workspace.kind === "timeline"
          ? null
          : intent.returnPoint ?? state.returnPoint,
      };
  }
}
