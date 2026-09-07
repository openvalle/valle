import type { JsonValue } from "@valle/engine";
import { LitElement, css, html, nothing, type TemplateResult } from "lit";

export interface MotionPropControlView {
  name: string;
  kind: "number" | "bool" | "string" | "select" | "readonly";
  value: unknown;
  min?: number;
  max?: number;
  step?: number;
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
}

export type MotionWorkspaceIntent =
  | { type: "prop"; name: string; value: { kind: string; value: JsonValue } }
  | { type: "phase"; key: string; value: number }
  | { type: "cue"; cue: string; key: string; value: number }
  | { type: "copy-props" }
  | { type: "return" };

export class StudioMotionWorkspace extends LitElement {
  static properties = {
    viewModel: { attribute: false },
  };

  static styles = css`
    :host { display: block; color: var(--text, #e8eaed); }
    :host([hidden]) { display: none; }
    .head { display: grid; gap: 6px; margin-bottom: 16px; }
    .source { color: var(--muted, #9aa0ab); overflow-wrap: anywhere; }
    .status { color: #6ee7b7; }
    .status.error { color: #fca5a5; }
    .fingerprint { color: var(--muted-2, #6b7280); font-size: 10px; overflow-wrap: anywhere; }
    section { margin: 0 0 18px; }
    h2 { margin: 0 0 8px; color: var(--muted-2, #6b7280); font-size: 10px; text-transform: uppercase; letter-spacing: .08em; }
    .row { display: grid; grid-template-columns: minmax(72px, auto) minmax(0, 1fr) 44px; gap: 8px; align-items: center; margin: 7px 0; }
    .row > span { color: var(--muted, #9aa0ab); overflow-wrap: anywhere; }
    input, select, button { box-sizing: border-box; border: 1px solid var(--line-strong, #333); border-radius: 6px; color: inherit; background: var(--panel-2, #1c1f26); font: inherit; padding: 5px 7px; }
    input[type="range"] { width: 100%; padding: 0; }
    input[type="checkbox"] { justify-self: start; }
    output { color: #67e8f9; text-align: right; font-variant-numeric: tabular-nums; }
    button { cursor: pointer; margin: 0 6px 6px 0; }
    .diagnostic, .mapping { border: 1px solid var(--line, #292d35); border-radius: 7px; margin: 6px 0; padding: 7px; cursor: pointer; overflow-wrap: anywhere; }
    .diagnostic { border-color: #71313d; color: #fecaca; }
    small { display: block; margin-top: 3px; color: var(--muted-2, #6b7280); }
  `;

  declare viewModel: MotionWorkspaceViewModel | null;

  constructor() {
    super();
    this.viewModel = null;
  }

  renderWorkspace(model: MotionWorkspaceViewModel): void {
    this.viewModel = model;
    this.requestUpdate();
  }

  protected render(): TemplateResult {
    const model = this.viewModel;
    if (!model) return html``;
    return html`
      <div class="head">
        <span class="status ${model.status === "error" ? "error" : ""}">
          ${model.status === "ok" ? `Synced · #${model.generation}` : `Compilation failed · #${model.generation}`}
        </span>
        <span class="source">${model.input}</span>
        <span class="fingerprint">${model.fingerprint ?? ""}</span>
        <div>
          ${model.canReturn ? html`<button id="returnTimeline" type="button" @click=${() => this.#emit({ type: "return" })}>Return to timeline</button>` : nothing}
          <button id="copyProps" type="button" @click=${() => this.#emit({ type: "copy-props" })}>Copy props JSON</button>
        </div>
      </div>
      <section><h2>Controls</h2>${model.props.map((control) => this.#prop(control))}</section>
      <section><h2>Prepare data${model.dataSource ? ` · ${model.dataSource}` : ""}</h2>
        ${model.data.length ? model.data.map((control) => html`
          <div class="mapping"><strong>data.${control.name}</strong><small>${control.kind}${control.maxItems == null ? "" : ` · max ${control.maxItems}`} · ${JSON.stringify(control.value)}</small></div>
        `) : html`<span>None</span>`}
      </section>
      <section><h2>Phase handles</h2>${model.phases.map((handle) => this.#handle(handle, "phase"))}</section>
      <section><h2>Cue handles</h2>${model.cues.map((handle) => this.#handle(handle, "cue"))}</section>
      <section><h2>Diagnostics</h2>
        ${model.diagnostics.length ? model.diagnostics.map((item) => html`
          <div class="diagnostic" @click=${() => this.#copy(item.location)}>${item.label}<small>${item.location} · Click to copy</small></div>
        `) : html`<span>None</span>`}
      </section>
      <section><h2>Source map</h2>
        ${model.mappings.map((item) => html`
          <div class="mapping" @click=${() => this.#copy(item.location)}>${item.key}<small>${item.location}${item.kind ? " · 3D object" : ""} · Click to copy</small></div>
        `)}
      </section>
    `;
  }

  #prop(control: MotionPropControlView): TemplateResult {
    if (control.kind === "number") return html`<label class="row"><span>props.${control.name}</span><input type="number" data-prop=${control.name} .value=${String(control.value ?? 0)} min=${control.min ?? ""} max=${control.max ?? ""} step=${control.step ?? ""} @input=${this.#propInput}><output></output></label>`;
    if (control.kind === "bool") return html`<label class="row"><span>props.${control.name}</span><input type="checkbox" data-prop=${control.name} .checked=${Boolean(control.value)} @change=${this.#propInput}><output></output></label>`;
    if (control.kind === "string") return html`<label class="row"><span>props.${control.name}</span><input type="text" data-prop=${control.name} .value=${String(control.value ?? "")} @input=${this.#propInput}><output></output></label>`;
    if (control.kind === "select") return html`<label class="row"><span>props.${control.name}</span><select data-prop=${control.name} .value=${String(control.value ?? "")} @change=${this.#propInput}>${(control.values ?? []).map((value) => html`<option value=${value}>${value}</option>`)}</select><output></output></label>`;
    return html`<div class="row"><span>props.${control.name}</span><span>${JSON.stringify(control.value)}</span><output></output></div>`;
  }

  #handle(handle: MotionHandleView & { cue?: string }, kind: "phase" | "cue"): TemplateResult {
    if (handle.readOnly) {
      return html`<div class="row"><span>${handle.label}</span><span>resolved</span><output>${handle.value}f</output></div>`;
    }
    return html`<label class="row"><span>${handle.label}</span><input type="range" min=${handle.min} max=${handle.max} .value=${String(handle.value)} data-key=${handle.key} data-cue=${handle.cue ?? ""} data-kind=${kind} @input=${this.#handleInput}><output>${handle.value}f</output></label>`;
  }

  #propInput = (event: Event): void => {
    const input = event.currentTarget as HTMLInputElement | HTMLSelectElement;
    const name = input.dataset.prop;
    if (!name) return;
    if (input instanceof HTMLInputElement && input.type === "number") this.#emit({ type: "prop", name, value: { kind: "number", value: Number(input.value) } });
    else if (input instanceof HTMLInputElement && input.type === "checkbox") this.#emit({ type: "prop", name, value: { kind: "bool", value: input.checked } });
    else if (input instanceof HTMLSelectElement) this.#emit({ type: "prop", name, value: { kind: "enum", value: input.value } });
    else this.#emit({ type: "prop", name, value: { kind: "str", value: input.value } });
  };

  #handleInput = (event: Event): void => {
    const input = event.currentTarget as HTMLInputElement;
    input.parentElement?.querySelector("output")?.replaceChildren(`${input.value}f`);
    if (input.dataset.kind === "phase") this.#emit({ type: "phase", key: input.dataset.key ?? "", value: Number(input.value) });
    else this.#emit({ type: "cue", cue: input.dataset.cue ?? "", key: input.dataset.key ?? "", value: Number(input.value) });
  };

  #emit(intent: MotionWorkspaceIntent): void {
    this.dispatchEvent(new CustomEvent("motion-workspace-intent", { detail: intent, bubbles: true, composed: true }));
  }

  #copy(value: string): void {
    void navigator.clipboard?.writeText(value);
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
