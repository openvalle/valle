import { LitElement, css, html } from "lit";

import { hydrateIcons } from "../../shared/icons.ts";
import type { StudioBoot, StudioEvent, StudioHost } from "./host.ts";
import {
  initialStudioShellState,
  reduceStudioIntent,
  type PreviewStatusKind,
  type StudioIntent,
  type StudioShellState,
} from "./shell-state.ts";

const PREVIEW_LABELS: Record<PreviewStatusKind, string> = {
  loading: "Loading",
  ready: "Preview up to date",
  updating: "Updating preview",
  error: "Preview failed · Details",
  unavailable: "Preview unavailable",
  empty: "No preview",
};

interface StudioProblem {
  severity: "warning" | "error";
  message: string;
  action?: "retry" | "conflict";
}

/** A compiler diagnostic usually starts with `file:line:column`; show that part separately. */
function splitLocation(message: string): { location: string | null; text: string } {
  const match = /^(\S+:\d+:\d+)\s+(.*)$/su.exec(message);
  return match ? { location: match[1]!, text: match[2]! } : { location: null, text: message };
}

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
  #problemsOpen = false;
  #conflictDismissed = false;
  #resizeObserver: ResizeObserver | null = null;

  constructor() {
    super();
    this.boot = null;
    this.shellState = initialStudioShellState;
  }

  get host(): StudioHost | null {
    return this.#host;
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
    const undoBtn = this.#req<HTMLButtonElement>("undoBtn");
    const redoBtn = this.#req<HTMLButtonElement>("redoBtn");
    const inspectorToggle = this.#req<HTMLButtonElement>("inspectorToggle");
    const focusPreview = this.#req<HTMLButtonElement>("focusPreview");
    const collapseTimeline = this.#req<HTMLButtonElement>("collapseTimeline");
    const splitV = this.#req<HTMLElement>("splitV");
    const splitH = this.#req<HTMLElement>("splitH");

    const problemsButton = this.#req<HTMLButtonElement>("problemsButton");
    problemsButton.addEventListener("click", (event) => {
      event.stopPropagation();
      this.#setProblemsOpen(!this.#problemsOpen);
    });
    this.#req<HTMLElement>("previewStatus").addEventListener("click", () => {
      if (this.shellState.previewStatus === "error") this.#setProblemsOpen(true);
    });
    document.addEventListener("pointerdown", (event) => {
      if (!this.#problemsOpen) return;
      const target = event.target as Node | null;
      if (target && (this.#req("problemsPanel").contains(target) || problemsButton.contains(target))) return;
      this.#setProblemsOpen(false);
    });
    window.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && this.#problemsOpen) this.#setProblemsOpen(false);
      // ⌥⌘I / ⌥⌘T (Alt+Ctrl elsewhere). Option changes `key` on macOS, so match the physical key.
      if (!event.altKey || !(event.metaKey || event.ctrlKey) || event.repeat) return;
      if (event.code === "KeyI") {
        event.preventDefault();
        inspectorToggle.click();
      } else if (event.code === "KeyT") {
        event.preventDefault();
        collapseTimeline.click();
      }
    });

    // Inspector tabs: Code is the source editor's own toggle; Properties closes it.
    const inspectorPanel = this.#req<HTMLElement>("inspectorPanel");
    const propertiesTab = this.#req<HTMLButtonElement>("propertiesTab");
    const codeTab = this.#req<HTMLButtonElement>("sourceToggle");
    propertiesTab.addEventListener("click", () => {
      if (inspectorPanel.classList.contains("source-open")) codeTab.click();
    });
    const syncTabs = () => {
      const code = inspectorPanel.classList.contains("source-open");
      propertiesTab.setAttribute("aria-selected", String(!code));
      codeTab.setAttribute("aria-selected", String(code));
    };
    new MutationObserver(syncTabs).observe(inspectorPanel, { attributes: true, attributeFilter: ["class"] });
    syncTabs();

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
    window.addEventListener("resize", () => this.#syncInspectorToggle());
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
    this.classList.toggle("hide-inspector", !state.inspectorVisible);
    this.classList.toggle("timeline-collapsed", state.timelineCollapsed);
    this.classList.toggle("preview-focus", state.previewFocus);

    this.#text("projectName", state.projectName);
    const badge = this.#req<HTMLElement>("sessionBadge");
    badge.textContent = state.session?.kind === "project"
      ? "Project"
      : state.session?.kind === "timeline-file"
        ? "Timeline"
        : state.session?.kind === "motion-file"
          ? "Motion"
          : "";
    badge.hidden = !state.session;

    this.#text("inspectorKind", state.session?.kind === "timeline-file" ? "Timeline file"
      : state.session?.kind === "motion-file" ? "Motion clip" : "Project");

    const undo = this.#req<HTMLButtonElement>("undoBtn");
    const redo = this.#req<HTMLButtonElement>("redoBtn");
    undo.disabled = !state.canUndo;
    redo.disabled = !state.canRedo;

    const saveStatus = this.#req<HTMLElement>("saveStatus");
    saveStatus.classList.toggle("dirty", state.dirty);
    saveStatus.classList.toggle("error", state.conflict);
    saveStatus.textContent = state.conflict
      ? (state.conflictMessage ?? "External change conflict")
      : state.sourceDirty ? "Unsaved source"
        : state.dirty ? "Unsaved changes" : "Saved";

    const previewStatus = this.#req<HTMLElement>("previewStatus");
    const previewText = this.#req<HTMLElement>("previewStatusText");
    previewStatus.className = `preview-status ${state.previewStatus === "error" ? "error"
      : state.previewStatus === "ready" ? "ready"
        : state.previewStatus === "updating" ? "warning" : "info"}`;
    previewText.textContent = PREVIEW_LABELS[state.previewStatus];
    previewStatus.title = state.previewMessage ?? "";

    if (!state.conflict) this.#conflictDismissed = false;
    const problems = this.#problems();
    const problemsButton = this.#req<HTMLButtonElement>("problemsButton");
    problemsButton.hidden = problems.length === 0;
    problemsButton.classList.toggle("error", problems.some((problem) => problem.severity === "error"));
    const problemsLabel = problems.length === 1 ? "1 problem" : `${problems.length} problems`;
    problemsButton.title = problemsLabel;
    problemsButton.setAttribute("aria-label", problemsLabel);
    this.#text("problemsCount", String(problems.length));
    if (problems.length === 0 && this.#problemsOpen) this.#problemsOpen = false;
    this.#renderProblems(problems);

    this.#syncInspectorToggle();
    const focusPreview = this.#req<HTMLButtonElement>("focusPreview");
    focusPreview.setAttribute("aria-pressed", String(state.previewFocus));
    const collapseTimeline = this.#req<HTMLButtonElement>("collapseTimeline");
    collapseTimeline.setAttribute(
      "aria-label",
      state.timelineCollapsed ? "Expand timeline" : "Collapse timeline",
    );
    collapseTimeline.title = `${state.timelineCollapsed ? "Expand" : "Collapse"} timeline (⌥⌘T)`;

    this.#text("timelineTitle", "Timeline");
    this.#text("timelineHint", "Drag the ruler to seek · Select a clip to edit");

    this.#persistPanelPrefs();
  }

  #syncInspectorToggle(): void {
    this.#req<HTMLButtonElement>("inspectorToggle").setAttribute("aria-pressed", String(
      innerWidth <= 780 ? this.classList.contains("mobile-inspector") : this.shellState.inspectorVisible,
    ));
  }

  #problems(): StudioProblem[] {
    const state = this.shellState;
    const problems: StudioProblem[] = [];
    if (state.conflict && !this.#conflictDismissed) {
      problems.push({ severity: "error", action: "conflict",
        message: state.conflictMessage ?? "Source changed outside Studio. Your draft is kept until you choose." });
    }
    if (state.previewStatus === "error") {
      problems.push({ severity: "error", action: "retry",
        message: state.previewMessage ?? "Preview failed. Showing the last successful result." });
    } else if (state.previewStatus === "unavailable") {
      problems.push({ severity: "warning", message: state.previewMessage ?? "Preview is unavailable for this session." });
    }
    for (const message of state.diagnostics) problems.push({ severity: "warning", message });
    return problems;
  }

  #setProblemsOpen(open: boolean): void {
    this.#problemsOpen = open && this.#problems().length > 0;
    this.#renderProblems(this.#problems());
  }

  #renderProblems(problems: readonly StudioProblem[]): void {
    const panel = this.#req<HTMLElement>("problemsPanel");
    const button = this.#req<HTMLButtonElement>("problemsButton");
    button.setAttribute("aria-expanded", String(this.#problemsOpen));
    panel.hidden = !this.#problemsOpen;
    if (!this.#problemsOpen) return;
    panel.replaceChildren();
    const heading = document.createElement("h2");
    heading.textContent = problems.length === 1 ? "1 problem" : `${problems.length} problems`;
    panel.append(heading);
    for (const problem of problems) {
      const item = document.createElement("div");
      item.className = `problem-item ${problem.severity}`;
      const icon = document.createElement("span");
      icon.dataset.icon = "alert";
      const { location, text } = splitLocation(problem.message);
      const body = document.createElement("div");
      if (location) {
        const loc = document.createElement("div");
        loc.className = "problem-loc";
        loc.textContent = location;
        body.append(loc);
      }
      const message = document.createElement("div");
      message.className = "problem-msg";
      message.textContent = text;
      body.append(message);
      item.append(icon, body);
      if (problem.action) {
        const actions = document.createElement("div");
        actions.className = "problem-actions";
        const button = (label: string, primary: boolean, onClick: () => void) => {
          const element = document.createElement("button");
          element.type = "button";
          element.className = primary ? "button primary" : "button";
          element.textContent = label;
          element.addEventListener("click", onClick);
          actions.append(element);
        };
        if (problem.action === "retry") {
          button("Retry", true, () => {
            this.#setProblemsOpen(false);
            this.dispatchEvent(new CustomEvent("studio-preview-retry", { bubbles: true, composed: true }));
          });
        } else {
          if (this.shellState.session?.kind === "motion-file") {
            button("Discard and reload", false, () => {
              this.#setProblemsOpen(false);
              this.dispatchEvent(new CustomEvent("studio-discard-draft"));
            });
          }
          button("Keep draft", true, () => {
            this.#conflictDismissed = true;
            this.#applyShellChrome();
            this.#setProblemsOpen(false);
          });
        }
        body.append(actions);
      }
      panel.append(item);
    }
    hydrateIcons(panel);
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
