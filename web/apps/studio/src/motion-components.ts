import type { JsonValue } from "@valle/engine";
import { LitElement, html, type TemplateResult } from "lit";

export interface MotionPropControlView {
  name: string;
  kind: "number" | "bool" | "string" | "select" | "color" | "readonly";
  value: unknown;
  min?: number;
  max?: number;
  step?: number;
  unit?: string;
  values?: ReadonlyArray<string>;
}

export interface MotionDataControlView {
  name: string;
  kind: string;
  value: unknown;
  maxItems?: number;
}

export interface MotionHandleView {
  key: string;
  label: string;
  value: number;
  min: number;
  max: number;
  unit?: string;
  readOnly?: boolean;
}

export interface MotionMappingView {
  key: string;
  location: string;
  kind?: "object3d";
}

export interface MotionWorkspaceViewModel {
  input: string;
  generation: number;
  status: "ok" | "error";
  fingerprint?: string;
  props: ReadonlyArray<MotionPropControlView>;
  data: ReadonlyArray<MotionDataControlView>;
  dataSource?: string | null;
  phases: ReadonlyArray<MotionHandleView>;
  cues: ReadonlyArray<MotionHandleView & { cue: string }>;
  diagnostics: ReadonlyArray<{ label: string; location: string }>;
  mappings: ReadonlyArray<MotionMappingView>;
  canReturn: boolean;
  selectedLocation?: string | null;
}

export type MotionWorkspaceIntent =
  | { type: "prop"; name: string; value: { kind: string; value: JsonValue } }
  | { type: "prop-end"; name: string }
  | { type: "phase"; key: string; value: number }
  | { type: "cue"; cue: string; key: string; value: number }
  | { type: "copy-props" }
  | { type: "return" };

export class StudioMotionWorkspace extends LitElement {
  static properties = {
    viewModel: { attribute: false },
  };

  declare viewModel: MotionWorkspaceViewModel | null;

  constructor() {
    super();
    this.viewModel = null;
  }

  protected createRenderRoot(): this {
    return this;
  }

  renderWorkspace(viewModel: MotionWorkspaceViewModel): void {
    this.viewModel = viewModel;
    this.requestUpdate();
    this.performUpdate();
    this.#hydrateIcons();
  }

  protected render(): TemplateResult {
    const model = this.viewModel;
    if (!model) return html``;
    if (model.status === "error") {
      return html`
        <div class="empty-inspector">
          <h2>Motion failed</h2>
          <p>${model.input}</p>
        </div>
        <section class="ins-section">
          ${model.diagnostics.map((item) => html`
            <div class="ins-note error">${item.label}</div>
            <div class="ins-note">${item.location}</div>
          `)}
        </section>
      `;
    }

    return html`
      <div class="inspector-summary kind-motion">
        <span class="icon" data-icon="motion"></span>
        <div>
          <strong>Motion</strong>
          <small title=${model.input}>${model.input.split(/[\\/]/).pop()}</small>
        </div>
      </div>
      ${model.canReturn
        ? html`
            <button class="button full-width" type="button" id="returnTimeline" @click=${this.#return}>
              <span class="icon" data-icon="arrow-left"></span>
              Back to timeline
            </button>
          `
        : ""}

      ${model.props.length
        ? html`
            <section class="ins-section">
              <h2 class="section-label">Parameters</h2>
              ${model.props.map((prop) => this.#renderProp(prop))}
            </section>
          `
        : ""}

      ${model.phases.length
        ? html`
            <section class="ins-section">
              <h2 class="section-label">Phases</h2>
              ${model.phases.map((phase) => this.#renderHandle(phase, (value) => {
                this.#emit({ type: "phase", key: phase.key, value });
              }))}
            </section>
          `
        : ""}

      ${model.cues.length
        ? html`
            <section class="ins-section">
              <h2 class="section-label">Cues</h2>
              ${model.cues.map((cue) => this.#renderHandle(cue, (value) => {
                this.#emit({ type: "cue", cue: cue.cue, key: cue.key, value });
              }, cue.label))}
            </section>
          `
        : ""}

      ${model.selectedLocation
        ? html`
            <section class="ins-section">
              <h2 class="section-label">Selection</h2>
              <div class="ins-note">${model.selectedLocation}</div>
              <button class="button full-width" type="button" id="copyLocation"
                @click=${() => void navigator.clipboard?.writeText(model.selectedLocation ?? "")}>
                <span class="icon" data-icon="copy"></span>
                Copy source location
              </button>
            </section>
          `
        : ""}

      <button class="button full-width" type="button" id="copyProps" @click=${this.#copyProps}>
        <span class="icon" data-icon="copy"></span>
        Copy parameter JSON
      </button>

      ${model.diagnostics.length
        ? html`
            <section class="ins-section">
              <h2 class="section-label">Issues</h2>
              ${model.diagnostics.map((item) => html`
                <div class="ins-note warning">${item.label}</div>
              `)}
            </section>
          `
        : ""}

      ${model.data.length
        ? html`
            <details class="details">
              <summary>Prepared data</summary>
              <code>
                ${model.dataSource ? `source: ${model.dataSource}\n` : ""}
                ${model.data.map((item) => `${item.name}: ${safeJson(item.value)}`).join("\n")}
              </code>
            </details>
          `
        : ""}

      <details class="details">
        <summary>Technical details</summary>
        <code>
          generation: ${model.generation}
          ${model.fingerprint ? `\nartifact: ${model.fingerprint}` : ""}
          ${model.mappings.length
            ? `\n\n${model.mappings.slice(0, 12).map((item) => `${item.key} → ${item.location}`).join("\n")}`
            : ""}
        </code>
      </details>
    `;
  }

  #renderProp(prop: MotionPropControlView): TemplateResult {
    if (prop.kind === "readonly") {
      return html`
        <div class="inspector-metric">
          <span>${prop.name}</span>
          <span>${formatValue(prop.value)}</span>
        </div>
      `;
    }
    if (prop.kind === "color") {
      const color = Array.isArray(prop.value) ? prop.value : [255, 255, 255, 255];
      const hex = typeof prop.value === "string" ? prop.value.slice(0, 7) : `#${color.slice(0, 3).map((channel) => Math.round(Number(channel)).toString(16).padStart(2, "0")).join("")}`;
      return html`<label class="field"><span class="field-label">${prop.name}</span>
        <input type="color" .value=${hex} @change=${(event: Event) => {
          const value = (event.currentTarget as HTMLInputElement).value.slice(1);
          this.#emit({ type: "prop", name: prop.name, value: { kind: "color", value: [0, 2, 4].map((i) => parseInt(value.slice(i, i + 2), 16)).concat(Number(color[3] ?? 255)) } });
        }} /></label>`;
    }
    if (prop.kind === "bool") {
      return html`
        <label class="field">
          <span class="field-label">${prop.name}</span>
          <input type="checkbox" data-prop=${prop.name} .checked=${Boolean(prop.value)}
            @change=${this.#onBool} />
        </label>
      `;
    }
    if (prop.kind === "select") {
      return html`
        <label class="field">
          <span class="field-label">${prop.name}</span>
          <select data-prop=${prop.name} .value=${String(prop.value)} @change=${this.#onSelect}>
            ${(prop.values ?? []).map((value) => html`<option value=${value}>${value}</option>`)}
          </select>
        </label>
      `;
    }
    if (prop.kind === "number") {
      return html`
        <label class="field">
          <span class="field-label">${prop.name}</span>
          <span class="number-field">
            <input type="number" data-prop=${prop.name} .value=${String(prop.value)}
              min=${prop.min ?? ""} max=${prop.max ?? ""} step=${prop.step ?? "any"}
              @change=${this.#onNumber} @blur=${this.#onNumber} @keydown=${this.#cancelField} />
            ${prop.unit ? html`<span class="field-unit">${prop.unit}</span>` : ""}
          </span>
        </label>
      `;
    }
    return html`
      <label class="field">
        <span class="field-label">${prop.name}</span>
        <input type="text" data-prop=${prop.name} .value=${String(prop.value ?? "")}
          @change=${this.#onString} @blur=${this.#onString} @keydown=${this.#cancelField} />
      </label>
    `;
  }

  #renderHandle(
    handle: MotionHandleView,
    commit: (value: number) => void,
    labelOverride?: string,
  ): TemplateResult {
    const label = labelOverride ?? handle.label;
    if (handle.readOnly) {
      return html`
        <div class="inspector-metric">
          <span>${label}</span>
          <span>${handle.value} ${handle.unit ?? "f"}</span>
        </div>
      `;
    }
    const submit = (event: Event) => {
      const input = event.currentTarget as HTMLInputElement;
      if (input.value === "" || !input.validity.valid) { input.setAttribute("aria-invalid", "true"); return; }
      input.removeAttribute("aria-invalid");
      const next = Number(input.value);
      if (Number.isFinite(next) && next !== handle.value) commit(Math.round(next));
    };
    return html`<label class="field"><span class="field-label">${label}</span><span class="number-field">
      <input type="number" .value=${String(handle.value)} min=${handle.min} max=${handle.max} step="1"
        @change=${submit} @blur=${submit} @keydown=${(event: KeyboardEvent) => {
          const input = event.currentTarget as HTMLInputElement;
          if (event.key === "Escape") { input.value = String(handle.value); input.removeAttribute("aria-invalid"); event.stopPropagation(); }
          if (event.key === "Enter" || event.key === "Escape") input.blur();
        }} /><span class="field-unit">${handle.unit ?? "f"}</span></span></label>`;
  }

  #emit(intent: MotionWorkspaceIntent): void {
    this.dispatchEvent(new CustomEvent("motion-workspace-intent", {
      detail: intent,
      bubbles: true,
      composed: true,
    }));
  }

  #onBool = (event: Event): void => {
    const input = event.currentTarget as HTMLInputElement;
    const name = input.dataset.prop;
    if (!name) return;
    this.#emit({ type: "prop", name, value: { kind: "bool", value: input.checked } });
    this.#emit({ type: "prop-end", name });
  };

  #onSelect = (event: Event): void => {
    const select = event.currentTarget as HTMLSelectElement;
    const name = select.dataset.prop;
    if (!name) return;
    this.#emit({ type: "prop", name, value: { kind: "select", value: select.value } });
    this.#emit({ type: "prop-end", name });
  };

  #onNumber = (event: Event): void => {
    const input = event.currentTarget as HTMLInputElement;
    const name = input.dataset.prop;
    if (!name || input.value === "" || !input.validity.valid) { input.setAttribute("aria-invalid", "true"); return; }
    input.removeAttribute("aria-invalid");
    const value = Number(input.value);
    if (!Number.isFinite(value)) return;
    this.#emit({ type: "prop", name, value: { kind: "number", value } });
  };

  #onString = (event: Event): void => {
    const input = event.currentTarget as HTMLInputElement;
    const name = input.dataset.prop;
    if (!name) return;
    this.#emit({ type: "prop", name, value: { kind: "string", value: input.value } });
  };

  #cancelField = (event: KeyboardEvent): void => {
    const input = event.currentTarget as HTMLInputElement;
    if (event.key === "Escape") {
      const prop = this.viewModel?.props.find((p) => p.name === input.dataset.prop);
      if (prop) input.value = String(prop.value ?? "");
      input.removeAttribute("aria-invalid"); input.blur(); event.stopPropagation();
    } else if (event.key === "Enter") input.blur();
  };

  #onPropEnd = (event: Event): void => {
    const input = event.currentTarget as HTMLInputElement;
    const name = input.dataset.prop;
    if (name) this.#emit({ type: "prop-end", name });
  };

  #copyProps = (): void => {
    this.#emit({ type: "copy-props" });
  };

  #return = (): void => {
    this.#emit({ type: "return" });
  };

  #hydrateIcons(): void {
    this.querySelectorAll<HTMLElement>("[data-icon]").forEach((element) => {
      if (element.firstElementChild?.tagName.toLowerCase() === "svg") return;
      const name = element.dataset.icon ?? "motion";
      const paths: Record<string, string> = {
        motion: "m12 3 9 5-9 5-9-5 9-5Zm-9 9 9 5 9-5m-18 5 9 5 9-5",
        "arrow-left": "m10 5-7 7 7 7M3 12h18",
        copy: "M8 8h13v13H8V8ZM16 8V3H3v13h5",
      };
      element.innerHTML = `<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="${paths[name] ?? paths.motion}"/></svg>`;
    });
  }
}

function formatValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean" || value == null) {
    return String(value);
  }
  return safeJson(value);
}

function safeJson(value: unknown): string {
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

if (!customElements.get("studio-motion-workspace")) {
  customElements.define("studio-motion-workspace", StudioMotionWorkspace);
}

declare global {
  interface HTMLElementTagNameMap {
    "studio-motion-workspace": StudioMotionWorkspace;
  }
}
