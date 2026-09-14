import { LitElement, css, html } from "lit";

import { hydrateIcons } from "../../shared/icons.ts";
import type { StudioBoot, StudioEvent, StudioHost } from "./host.ts";
import {
  initialStudioShellState,
  reduceStudioIntent,
  type PreviewStatusKind,
  type StudioIntent,
  type StudioShellState,
  type StudioWorkspace,
} from "./shell-state.ts";

const PREVIEW_LABELS: Record<PreviewStatusKind, string> = {
  loading: "Loading…",
  ready: "Preview up to date",
  updating: "Updating preview…",
  error: "Preview failed",
  unavailable: "Preview unavailable",
  empty: "No preview",
};

export class ValleStudioApp extends LitElement {
  static properties = {
    boot: { attribute: false },
    shellState: { state: true },
  };

  static styles = css`
    :host {
      display: contents;
    }
  `;

  declare boot: StudioBoot | null;
  declare shellState: StudioShellState;

  #host: StudioHost | null = null;
  #unsubscribe: (() => void) | null = null;
  #splitGesture: { axis: "x" | "y"; pointerId: number } | null = null;
  #resizeObserver: ResizeObserver | null = null;

  constructor() {
    super();
    this.boot = null;
    this.shellState = initialStudioShellState;
  }

  get host(): StudioHost | null {
    return this.#host;
  }

  get workspace(): StudioWorkspace {
    return this.shellState.workspace;
  }

  get root(): HTMLElement {
    return this;
  }

  async initialize(host: StudioHost): Promise<void> {
    this.#unsubscribe?.();
    this.#host = host;
    this.boot = await host.load();
    this.shellState = {
      ...initialStudioShellState,
      session: this.boot.session,
      projectName: this.#projectLabel(this.boot),
    };
    this.dataset.sessionKind = this.boot.session.kind;
    this.dataset.workspace = this.shellState.workspace.kind;
    this.#unsubscribe = host.subscribe((event) => this.#onHostEvent(event));
    try {
      const preferences = JSON.parse(localStorage.getItem("valle.studio.panels") ?? "{}");
      if (typeof preferences.inspectorVisible === "boolean") this.shellState = { ...this.shellState, inspectorVisible: preferences.inspectorVisible };
      if (typeof preferences.timelineCollapsed === "boolean") this.shellState = { ...this.shellState, timelineCollapsed: preferences.timelineCollapsed };
      if (typeof preferences.width === "number") this.style.setProperty("--inspector-width", `${Math.max(280, Math.min(440, preferences.width))}px`);
      if (typeof preferences.height === "number") this.style.setProperty("--timeline-height", `${Math.max(120, Math.min(innerHeight - 320, preferences.height))}px`);
    } catch { /* Storage is optional. */ }
    if (innerHeight <= 560) this.shellState = { ...this.shellState, timelineCollapsed: true };
    hydrateIcons(this);
    this.#bindChrome();
    this.#applyShellChrome();
    this.dispatchEvent(new CustomEvent("studio-ready", {
      detail: { boot: this.boot },
      bubbles: true,
      composed: true,
    }));
  }

  dispatchIntent(intent: StudioIntent): void {
    this.shellState = reduceStudioIntent(this.shellState, intent);
    this.dataset.workspace = this.shellState.workspace.kind;
    this.#applyShellChrome();
    if (intent.type === "panel") this.#savePreferences();
    this.dispatchEvent(new CustomEvent("studio-statechange", {
      detail: { state: this.shellState, intent },
      bubbles: true,
      composed: true,
    }));
  }

  connectedCallback(): void {
    super.connectedCallback();
    this.addEventListener("studio-intent", this.#onIntent as EventListener);
  }

  disconnectedCallback(): void {
    this.removeEventListener("studio-intent", this.#onIntent as EventListener);
    this.#unsubscribe?.();
    this.#unsubscribe = null;
    this.#resizeObserver?.disconnect();
    this.#resizeObserver = null;
    super.disconnectedCallback();
  }

  protected render() {
    return html`<slot></slot>`;
  }

  #projectLabel(boot: StudioBoot): string {
    if (boot.session.kind === "project") return boot.session.projectId;
    if (boot.session.kind === "timeline-file") {
      return boot.session.input.split(/[\\/]/).pop() || boot.session.input;
    }
    if (boot.session.kind === "motion-file") {
      return boot.session.input.split(/[\\/]/).pop() || boot.session.input;
    }
    return "Studio";
  }

  #onIntent = (event: CustomEvent<StudioIntent>): void => {
    event.stopPropagation();
    this.dispatchIntent(event.detail);
  };

  #onHostEvent(event: StudioEvent): void {
    this.dispatchEvent(new CustomEvent("studio-host-event", {
      detail: event,
      bubbles: true,
      composed: true,
    }));
  }

  #savePreferences(): void {
    try { localStorage.setItem("valle.studio.panels", JSON.stringify({
      inspectorVisible: this.shellState.inspectorVisible, timelineCollapsed: this.shellState.timelineCollapsed,
      width: parseFloat(this.style.getPropertyValue("--inspector-width")) || undefined,
      height: parseFloat(this.style.getPropertyValue("--timeline-height")) || undefined,
    })); } catch { /* Storage is optional. */ }
  }

  #bindChrome(): void {
    this.#req<HTMLButtonElement>("stageFit").addEventListener("click", (event) => {
      const full = this.#req<HTMLElement>("stageArea").classList.toggle("full-size");
      (event.currentTarget as HTMLButtonElement).textContent = full ? "100%" : "Fit";
    });
    const timelineMode = this.#req<HTMLButtonElement>("timelineMode");
    const motionMode = this.#req<HTMLButtonElement>("motionMode");
    const undoBtn = this.#req<HTMLButtonElement>("undoBtn");
    const redoBtn = this.#req<HTMLButtonElement>("redoBtn");
    const inspectorToggle = this.#req<HTMLButtonElement>("inspectorToggle");
    const focusPreview = this.#req<HTMLButtonElement>("focusPreview");
    const collapseTimeline = this.#req<HTMLButtonElement>("collapseTimeline");
    const splitV = this.#req<HTMLElement>("splitV");
    const splitH = this.#req<HTMLElement>("splitH");

    timelineMode.addEventListener("click", () => {
      if (this.shellState.workspace.kind !== "timeline") {
        this.dispatchEvent(new CustomEvent("studio-return-timeline", {
          bubbles: true,
          composed: true,
        }));
      }
    });

    motionMode.addEventListener("click", () => {
      this.dispatchEvent(new CustomEvent("studio-open-motion", {
        bubbles: true,
        composed: true,
      }));
    });

    undoBtn.addEventListener("click", () => {
      this.dispatchEvent(new CustomEvent("studio-history-intent", {
        detail: { type: "undo" },
        bubbles: true,
        composed: true,
      }));
    });

    redoBtn.addEventListener("click", () => {
      this.dispatchEvent(new CustomEvent("studio-history-intent", {
        detail: { type: "redo" },
        bubbles: true,
        composed: true,
      }));
    });

    inspectorToggle.addEventListener("click", () => {
      if (innerWidth <= 780) {
        this.classList.toggle("mobile-inspector");
        inspectorToggle.setAttribute("aria-pressed", String(this.classList.contains("mobile-inspector")));
        return;
      }
      this.dispatchIntent({ type: "panel", inspectorVisible: !this.shellState.inspectorVisible });
    });
    window.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && this.classList.contains("mobile-inspector")) {
        this.classList.remove("mobile-inspector");
        inspectorToggle.setAttribute("aria-pressed", "false");
      }
    });

    focusPreview.addEventListener("click", () => {
      this.dispatchIntent({ type: "panel", previewFocus: !this.shellState.previewFocus });
    });

    collapseTimeline.addEventListener("click", () => {
      this.dispatchIntent({ type: "panel", timelineCollapsed: !this.shellState.timelineCollapsed });
    });

    for (const [element, axis] of [
      [splitV, "x"] as const,
      [splitH, "y"] as const,
    ]) {
      element.addEventListener("pointerdown", (event) => {
        if (event.button !== 0) return;
        this.#splitGesture = { axis, pointerId: event.pointerId };
        element.classList.add("dragging");
        element.setPointerCapture(event.pointerId);
        event.preventDefault();
      });
      element.addEventListener("pointermove", (event) => {
        if (this.#splitGesture?.axis !== axis || this.#splitGesture.pointerId !== event.pointerId) return;
        if (axis === "x") {
          const width = Math.min(440, Math.max(280, innerWidth - event.clientX));
          this.style.setProperty("--inspector-width", `${width}px`);
        } else {
          const height = Math.min(
            Math.max(120, innerHeight - 320),
            Math.max(120, innerHeight - event.clientY - 8),
          );
          this.style.setProperty("--timeline-height", `${height}px`);
        }
      });
      const stop = () => {
        if (!this.#splitGesture) return;
        this.#splitGesture = null;
        element.classList.remove("dragging");
        this.#savePreferences();
      };
      element.addEventListener("pointerup", stop);
      element.addEventListener("pointercancel", stop);
      element.addEventListener("dblclick", () => {
        this.style.removeProperty(axis === "x" ? "--inspector-width" : "--timeline-height");
        this.#savePreferences();
      });
    }

    this.#resizeObserver = new ResizeObserver(() => {
      this.dispatchEvent(new CustomEvent("studio-layout", {
        detail: {
          width: this.clientWidth,
          height: this.clientHeight,
        },
        bubbles: true,
        composed: true,
      }));
    });
    this.#resizeObserver.observe(this);
  }

  #applyShellChrome(): void {
    const state = this.shellState;
    this.dataset.workspace = state.workspace.kind;
    this.classList.toggle("motion-workspace", state.workspace.kind === "motion");
    this.classList.toggle("hide-inspector", !state.inspectorVisible);
    this.classList.toggle("timeline-collapsed", state.timelineCollapsed);
    this.classList.toggle("preview-focus", state.previewFocus);

    this.#text("projectName", state.projectName);
    const badge = this.#req<HTMLElement>("sessionBadge");
    badge.textContent = state.session?.kind === "project"
      ? "Project"
      : state.session?.kind === "timeline-file"
        ? "Timeline file"
        : state.session?.kind === "motion-file"
          ? "Motion file"
          : "";
    badge.hidden = !state.session;

    const crumb = this.#req<HTMLElement>("workspaceCrumb");
    if (state.workspace.kind === "motion" && state.session?.kind !== "motion-file") {
      crumb.hidden = false;
      crumb.textContent = state.workspace.source
        ? ` / ${state.workspace.source.split(/[\\/]/).pop()}`
        : " / Motion";
    } else {
      crumb.hidden = true;
      crumb.textContent = "";
    }

    const timelineTab = this.#req<HTMLButtonElement>("timelineMode");
    const motionTab = this.#req<HTMLButtonElement>("motionMode");
    const isTimeline = state.workspace.kind === "timeline";
    this.#text("inspectorKind", isTimeline ? state.session?.kind === "timeline-file" ? "Timeline file" : "Project" : "Motion");
    timelineTab.classList.toggle("active", isTimeline);
    motionTab.classList.toggle("active", !isTimeline);
    timelineTab.disabled = state.session?.kind === "motion-file";
    motionTab.disabled = isTimeline && !state.canOpenMotion;
    timelineTab.setAttribute("aria-pressed", String(isTimeline));
    motionTab.setAttribute("aria-pressed", String(!isTimeline));

    const undo = this.#req<HTMLButtonElement>("undoBtn");
    const redo = this.#req<HTMLButtonElement>("redoBtn");
    undo.disabled = !state.canUndo;
    redo.disabled = !state.canRedo;

    const saveStatus = this.#req<HTMLElement>("saveStatus");
    saveStatus.classList.toggle("dirty", state.dirty);
    saveStatus.classList.toggle("error", state.conflict);
    saveStatus.textContent = state.conflict
      ? (state.conflictMessage ?? "External change conflict")
      : state.session?.kind === "project" || state.session?.kind === "timeline-file"
        ? state.dirty ? "Unsaved changes" : "Saved"
        : state.dirty ? "Modified parameters · source unchanged" : "Temporary parameters · source unchanged";

    const previewStatus = this.#req<HTMLElement>("previewStatus");
    const previewText = this.#req<HTMLElement>("previewStatusText");
    previewStatus.className = "preview-status";
    if (state.previewStatus === "error") previewStatus.classList.add("error");
    else if (state.previewStatus === "updating" || state.previewStatus === "unavailable") {
      previewStatus.classList.add("warning");
    } else if (state.previewStatus === "loading" || state.previewStatus === "empty") {
      previewStatus.classList.add("info");
    }
    previewText.textContent = PREVIEW_LABELS[state.previewStatus];
    previewStatus.title = state.previewMessage ?? "";

    const banner = this.#req<HTMLElement>("stageBanner");
    const showBanner = state.conflict
      || state.previewStatus === "error"
      || state.previewStatus === "updating"
      || state.previewStatus === "unavailable";
    banner.hidden = !showBanner;
    banner.classList.toggle("error", state.previewStatus === "error" || state.conflict);
    if (showBanner) {
      const message = state.conflict
        ? (state.conflictMessage ?? "Source changed outside Studio. Draft is kept until you choose.")
        : state.previewStatus === "error"
          ? (state.previewMessage ?? "Preview failed. Showing the last successful result.")
          : state.previewStatus === "unavailable"
            ? (state.previewMessage ?? "Preview is unavailable for this session.")
            : (state.previewMessage ?? "Updating preview… showing the last successful result.");
      banner.innerHTML = "";
      const span = document.createElement("span");
      span.textContent = state.previewStatus === "error" ? "Preview failed. Showing the last successful result." : message;
      span.title = state.previewMessage ?? message;
      banner.append(span);
      if (state.conflict) {
        const keep = document.createElement("button");
        keep.type = "button";
        keep.textContent = "Keep draft";
        keep.addEventListener("click", () => { banner.hidden = true; });
        if (state.session?.kind === "motion-file") {
          const discard = document.createElement("button"); discard.type = "button"; discard.textContent = "Discard and reload";
          discard.addEventListener("click", () => this.dispatchEvent(new CustomEvent("studio-discard-draft")));
          banner.append(discard);
        }
        banner.append(keep);
      } else if (state.previewStatus === "error") {
        const retry = document.createElement("button");
        retry.type = "button";
        retry.textContent = "Retry";
        retry.addEventListener("click", () => {
          this.dispatchEvent(new CustomEvent("studio-preview-retry", {
            bubbles: true,
            composed: true,
          }));
        });
        banner.append(retry);
      }
    }

    const inspectorToggle = this.#req<HTMLButtonElement>("inspectorToggle");
    inspectorToggle.setAttribute("aria-pressed", String(innerWidth <= 780 ? this.classList.contains("mobile-inspector") : state.inspectorVisible));
    const focusPreview = this.#req<HTMLButtonElement>("focusPreview");
    focusPreview.setAttribute("aria-pressed", String(state.previewFocus));
    const collapseTimeline = this.#req<HTMLButtonElement>("collapseTimeline");
    collapseTimeline.setAttribute(
      "aria-label",
      state.timelineCollapsed ? "Expand timeline" : "Collapse timeline",
    );

    this.#text("timelineTitle", state.workspace.kind === "motion" ? "Phases & cues" : "Timeline");
    this.#text(
      "timelineHint",
      state.workspace.kind === "motion"
        ? "Adjust enter/exit · Jump to cue bounds"
        : "Drag the ruler to seek · Select a clip to edit",
    );

    this.#persistPanelPrefs();
  }

  #persistPanelPrefs(): void {
    try {
      const payload = JSON.stringify({
        inspectorVisible: this.shellState.inspectorVisible,
        timelineCollapsed: this.shellState.timelineCollapsed,
      });
      localStorage.setItem("valle.studio.panel", payload);
    } catch {
      // Preference persistence is optional.
    }
  }

  restorePanelPrefs(): void {
    try {
      const raw = localStorage.getItem("valle.studio.panel");
      if (!raw) return;
      const parsed = JSON.parse(raw) as { inspectorVisible?: boolean; timelineCollapsed?: boolean };
      this.dispatchIntent({
        type: "panel",
        inspectorVisible: parsed.inspectorVisible !== false,
        timelineCollapsed: parsed.timelineCollapsed === true,
      });
    } catch {
      // Ignore invalid preferences.
    }
  }

  #req<T extends HTMLElement>(id: string): T {
    const element = document.getElementById(id);
    if (!element) throw new Error(`missing required Studio element #${id}`);
    return element as T;
  }

  #text(id: string, value: string): void {
    const element = document.getElementById(id);
    if (element) element.textContent = value;
  }
}

if (!customElements.get("valle-studio-app")) {
  customElements.define("valle-studio-app", ValleStudioApp);
}

declare global {
  interface HTMLElementTagNameMap {
    "valle-studio-app": ValleStudioApp;
  }
}
