import { LitElement, css, html } from "lit";

import type { StudioBoot, StudioEvent, StudioHost } from "./host.ts";
import {
  initialStudioShellState,
  reduceStudioIntent,
  type StudioIntent,
  type StudioShellState,
  type StudioWorkspace,
} from "./shell-state.ts";

export class ValleStudioApp extends LitElement {
  static properties = {
    boot: { attribute: false },
    shellState: { state: true },
  };

  static styles = css`
    :host {
      display: grid;
      box-sizing: border-box;
      height: 100dvh;
      grid-template-columns: minmax(0, 1fr) var(--inspector-w, 300px);
      grid-template-rows: auto minmax(0, 1fr) auto minmax(120px, var(--tracks-h, 26dvh));
      gap: 10px;
      padding: 10px;
      overflow: hidden;
    }

    slot {
      display: contents;
    }
  `;

  declare boot: StudioBoot | null;
  declare shellState: StudioShellState;

  #host: StudioHost | null = null;
  #unsubscribe: (() => void) | null = null;

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

  async initialize(host: StudioHost): Promise<void> {
    this.#unsubscribe?.();
    this.#host = host;
    this.boot = await host.load();
    this.shellState = { ...initialStudioShellState, session: this.boot.session };
    this.dataset.sessionKind = this.boot.session.kind;
    this.dataset.workspace = this.shellState.workspace.kind;
    this.#unsubscribe = host.subscribe((event) => this.#onHostEvent(event));
    this.dispatchEvent(new CustomEvent("studio-ready", {
      detail: { boot: this.boot },
      bubbles: true,
      composed: true,
    }));
  }

  dispatchIntent(intent: StudioIntent): void {
    this.shellState = reduceStudioIntent(this.shellState, intent);
    this.dataset.workspace = this.shellState.workspace.kind;
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
    super.disconnectedCallback();
  }

  protected render() {
    return html`<slot></slot>`;
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
}

if (!customElements.get("valle-studio-app")) {
  customElements.define("valle-studio-app", ValleStudioApp);
}

declare global {
  interface HTMLElementTagNameMap {
    "valle-studio-app": ValleStudioApp;
  }
}

