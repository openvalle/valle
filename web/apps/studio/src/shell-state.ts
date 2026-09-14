import type { StudioSession } from "./host.ts";

export type StudioWorkspace =
  | { kind: "timeline" }
  | { kind: "motion"; clipId?: string; source: string };

export type PreviewStatusKind =
  | "loading"
  | "ready"
  | "updating"
  | "error"
  | "unavailable"
  | "empty";

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
  canOpenMotion?: boolean;
  dirty: boolean;
  conflict: boolean;
  conflictMessage: string | null;
  diagnostics: ReadonlyArray<string>;
  returnPoint: StudioReturnPoint | null;
  previewStatus: PreviewStatusKind;
  previewMessage: string | null;
  canUndo: boolean;
  canRedo: boolean;
  inspectorVisible: boolean;
  timelineCollapsed: boolean;
  previewFocus: boolean;
  projectName: string;
}

export type StudioIntent =
  | { type: "select"; clipId: string | null; canOpenMotion?: boolean }
  | { type: "dirty"; value: boolean }
  | { type: "conflict"; value: boolean; message?: string | null }
  | { type: "workspace"; workspace: StudioWorkspace; returnPoint?: StudioReturnPoint }
  | { type: "diagnostics"; messages: ReadonlyArray<string> }
  | { type: "preview-status"; status: PreviewStatusKind; message?: string | null }
  | { type: "history"; canUndo: boolean; canRedo: boolean }
  | { type: "panel"; inspectorVisible?: boolean; timelineCollapsed?: boolean; previewFocus?: boolean }
  | { type: "project-name"; name: string };

export const initialStudioShellState: StudioShellState = {
  session: null,
  workspace: { kind: "timeline" },
  selectedClipId: null,
  dirty: false,
  conflict: false,
  conflictMessage: null,
  diagnostics: [],
  returnPoint: null,
  previewStatus: "loading",
  previewMessage: null,
  canUndo: false,
  canRedo: false,
  inspectorVisible: true,
  timelineCollapsed: false,
  previewFocus: false,
  projectName: "Studio",
};

export function reduceStudioIntent(
  state: StudioShellState,
  intent: StudioIntent,
): StudioShellState {
  switch (intent.type) {
    case "select":
      return { ...state, selectedClipId: intent.clipId, canOpenMotion: intent.canOpenMotion ?? false };
    case "dirty":
      return { ...state, dirty: intent.value };
    case "conflict":
      return {
        ...state,
        conflict: intent.value,
        conflictMessage: intent.message ?? (intent.value ? state.conflictMessage : null),
      };
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
    case "preview-status":
      return {
        ...state,
        previewStatus: intent.status,
        previewMessage: intent.message ?? null,
      };
    case "history":
      return { ...state, canUndo: intent.canUndo, canRedo: intent.canRedo };
    case "panel":
      return {
        ...state,
        inspectorVisible: intent.inspectorVisible ?? state.inspectorVisible,
        timelineCollapsed: intent.timelineCollapsed ?? state.timelineCollapsed,
        previewFocus: intent.previewFocus ?? state.previewFocus,
      };
    case "project-name":
      return { ...state, projectName: intent.name };
  }
}
