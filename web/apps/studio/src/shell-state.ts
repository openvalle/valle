import type { StudioSession } from "./host.ts";

export type PreviewStatusKind =
  | "loading"
  | "ready"
  | "updating"
  | "error"
  | "unavailable"
  | "empty";

export interface StudioShellState {
  session: StudioSession | null;
  selectedClipId: string | null;
  dirty: boolean;
  sourceDirty: boolean;
  conflict: boolean;
  conflictMessage: string | null;
  diagnostics: ReadonlyArray<string>;
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
  | { type: "select"; clipId: string | null }
  | { type: "dirty"; value: boolean }
  | { type: "source-dirty"; value: boolean }
  | { type: "conflict"; value: boolean; message?: string | null }
  | { type: "diagnostics"; messages: ReadonlyArray<string> }
  | { type: "preview-status"; status: PreviewStatusKind; message?: string | null }
  | { type: "history"; canUndo: boolean; canRedo: boolean }
  | { type: "panel"; inspectorVisible?: boolean; timelineCollapsed?: boolean; previewFocus?: boolean }
  | { type: "project-name"; name: string };

export const initialStudioShellState: StudioShellState = {
  session: null,
  selectedClipId: null,
  dirty: false,
  sourceDirty: false,
  conflict: false,
  conflictMessage: null,
  diagnostics: [],
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
      return { ...state, selectedClipId: intent.clipId };
    case "dirty":
      return { ...state, dirty: intent.value };
    case "source-dirty":
      return { ...state, sourceDirty: intent.value };
    case "conflict":
      return {
        ...state,
        conflict: intent.value,
        conflictMessage: intent.message ?? (intent.value ? state.conflictMessage : null),
      };
    case "diagnostics":
      return { ...state, diagnostics: [...intent.messages] };
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
