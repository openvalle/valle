// Studio: what `valle motion|timeline|project studio` opens, played by the SDK and edited in
// place. Unsaved changes are drafts the preview plays; saving writes the sources first, then the
// timeline, each over the file as it was read, so a change made outside is never lost. Exports
// are native renders the service runs.

import { css, html, LitElement, nothing } from "lit";
import { createPlayer, type Player } from "../../../packages/engine/src/index.ts";

import {
  Conflict,
  fetching,
  type Layout,
  link,
  type Listed,
  type Session,
  service,
} from "./service.ts";
import "./timeline-view.ts";
import type { TimelineView } from "./timeline-view.ts";

/** Files the source panel edits as text. */
const TEXT = /\.(tsx?|json|css|txt|md|srt|vtt|lrc|csv|glsl|wgsl)$/;

/** An edit undone and redone: a file's text before and after. */
type Step = { path: string; before: string; after: string };

/** A render the service runs: how far it is, and what it made. */
type Job = { id: number; done: number; frames: number; result?: Record<string, any> };

export class ValleStudio extends LitElement {
  static styles = css`
    :host {
      display: grid;
      grid-template: "top top top" auto "left stage right" 1fr "left bottom right" auto / 240px 1fr 320px;
      height: 100vh;
      background: #0f172a;
      color: #e2e8f0;
      font:
        13px system-ui,
        sans-serif;
    }
    header {
      grid-area: top;
      display: flex;
      gap: 8px;
      align-items: center;
      padding: 6px 10px;
      border-bottom: 1px solid #1e293b;
    }
    header .name {
      font-weight: 600;
      margin-right: auto;
    }
    aside {
      overflow: auto;
      padding: 8px;
      border-right: 1px solid #1e293b;
    }
    aside.right {
      grid-area: right;
      border-right: 0;
      border-left: 1px solid #1e293b;
      display: flex;
      flex-direction: column;
      gap: 8px;
    }
    aside.left {
      grid-area: left;
    }
    h3 {
      margin: 10px 0 4px;
      font-size: 11px;
      text-transform: uppercase;
      color: #64748b;
    }
    .file {
      display: block;
      width: 100%;
      text-align: left;
      background: none;
      border: 0;
      color: inherit;
      padding: 2px 4px;
      cursor: pointer;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .file.open,
    .file:hover {
      background: #1e293b;
    }
    .file small {
      color: #64748b;
    }
    .stage {
      grid-area: stage;
      position: relative;
      display: flex;
      flex-direction: column;
      min-height: 0;
    }
    .canvas {
      flex: 1;
      min-height: 0;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 8px;
    }
    .canvas canvas {
      max-width: 100%;
      max-height: 100%;
      box-shadow: 0 0 0 1px #334155;
    }
    .error {
      position: absolute;
      inset: 8px 8px auto;
      background: #7f1d1d;
      border-radius: 4px;
      padding: 8px;
      white-space: pre-wrap;
      font-family: ui-monospace, monospace;
    }
    .transport {
      display: flex;
      gap: 8px;
      align-items: center;
      padding: 4px 8px;
      border-top: 1px solid #1e293b;
    }
    .transport input[type="range"] {
      flex: 1;
    }
    .bottom {
      grid-area: bottom;
      border-top: 1px solid #1e293b;
      padding: 0 10px 6px;
      max-height: 40vh;
      overflow: auto;
    }
    textarea {
      flex: 1;
      min-height: 200px;
      background: #020617;
      color: #e2e8f0;
      border: 1px solid #334155;
      font:
        12px/1.4 ui-monospace,
        monospace;
      tab-size: 2;
    }
    button,
    select,
    input {
      font: inherit;
    }
    button {
      background: #1e293b;
      color: inherit;
      border: 1px solid #334155;
      border-radius: 4px;
      padding: 3px 8px;
      cursor: pointer;
    }
    button:disabled {
      opacity: 0.5;
    }
    .banner {
      background: #78350f;
      padding: 6px;
      border-radius: 4px;
    }
    label.field {
      display: grid;
      grid-template-columns: 100px 1fr;
      gap: 6px;
      align-items: center;
      margin: 3px 0;
    }
    .home {
      grid-column: 1 / -1;
      grid-row: 2 / -1;
      padding: 40px;
      max-width: 640px;
    }
  `;

  session: Session | undefined;
  /** The files read, as they are on disk, and the drafts of those changed here. */
  #files = new Map<string, { text: string; sha256: string | null }>();
  #drafts = new Map<string, string>();
  #conflicts = new Set<string>();
  #listed: Listed[] = [];
  #history: Record<string, any>[] = [];
  #jobs = new Map<number, Job>();
  /** The file the source panel shows. */
  #open = "";
  /** The entry: the scene or timeline played; a project's can be another of its timelines. */
  #entry = "";
  player: Player | undefined;
  #error = "";
  layout: Layout | undefined;
  #selected: string | undefined;
  #undo: Step[] = [];
  #redo: Step[] = [];
  #status = "";
  #time = 0;
  /** Each opening of the player, so one superseded is let go. */
  #opening = 0;
  #reload: ReturnType<typeof setTimeout> | undefined;
  /** The props the preview of a scene is played with. */
  #values: Record<string, unknown> = {};

  connectedCallback(): void {
    super.connectedCallback();
    addEventListener("keydown", this.#key);
    void this.#start();
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    removeEventListener("keydown", this.#key);
  }

  async #start(): Promise<void> {
    this.session = await service.session();
    service.events((event) => void this.#event(event));
    if (this.session.kind !== "home") {
      await this.#load(this.session.entry!);
    }
    this.requestUpdate();
  }

  /** Opens `entry`: its text, the folder's files, and a player of it. */
  async #load(entry: string): Promise<void> {
    this.#entry = this.#open = entry;
    this.#selected = undefined;
    await this.#read(entry);
    await this.#refresh();
    await this.#play();
  }

  /** The folder's files, and a project's snapshots. */
  async #refresh(): Promise<void> {
    this.#listed = await service.list();
    if (this.session?.kind === "project") {
      this.#history = (await service.project(["history"])).snapshots ?? [];
    }
    this.requestUpdate();
  }

  async #read(path: string): Promise<void> {
    const read = await service.read(path);
    this.#files.set(path, read ?? { text: "", sha256: null });
  }

  /** A file's text here: its draft, else as read. */
  text(path: string): string {
    return this.#drafts.get(path) ?? this.#files.get(path)?.text ?? "";
  }

  get timeline(): boolean {
    return this.#entry.endsWith(".json");
  }

  /** A new player of the entry, its drafts given in place of the files. */
  async #play(): Promise<void> {
    const opening = ++this.#opening;
    const time = this.player?.currentTime ?? 0;
    await this.player?.close();
    this.player = undefined;
    const canvas = document.createElement("canvas");
    const stage = this.renderRoot.querySelector(".canvas");
    stage?.replaceChildren(canvas);
    const files: Record<string, Blob> = {};
    for (const [path, text] of this.#drafts) {
      files[path] = new Blob([text]);
    }
    if (this.timeline) {
      await this.#lay();
    }
    try {
      const player = await createPlayer({
        canvas,
        runtime: "/valle/",
        entry: this.#entry,
        base: "/api/files/",
        fonts: "/api/fonts/",
        fetch: fetching,
        files,
      });
      if (opening !== this.#opening) {
        await player.close();
        return;
      }
      this.player = player;
      this.#error = "";
      player.on("time", ({ time }) => {
        this.#time = time;
        this.requestUpdate();
      });
      player.on("error", (error) => {
        this.#error = error.message;
        this.requestUpdate();
      });
      player.seek(time);
    } catch (error) {
      if (opening === this.#opening) {
        this.#error = (error as Error).message;
      }
    }
    this.requestUpdate();
  }

  /** Where the timeline's clips are, from its text here. */
  async #lay(): Promise<void> {
    try {
      this.layout = (await service.edit(this.text(this.#entry))).layout;
    } catch (error) {
      this.layout = undefined;
      this.#error = (error as Error).message;
    }
  }

  /** Plays the drafts once typing stops. */
  #later(): void {
    clearTimeout(this.#reload);
    this.#reload = setTimeout(() => void this.#play(), 400);
  }

  /** Does `op` to the timeline open: a draft of it, which can be undone. */
  async edit(op: unknown): Promise<void> {
    const path = this.#entry;
    const before = this.text(path);
    try {
      const { text, layout } = await service.edit(before, op);
      this.#change({ path, before, after: text });
      this.layout = layout;
      this.#undo.push({ path, before, after: text });
      this.#redo = [];
      this.#status = "";
      await this.#play();
    } catch (error) {
      this.#status = (error as Error).message;
      this.requestUpdate();
    }
  }

  #change(step: { path: string; after: string }): void {
    if (step.after === this.#files.get(step.path)?.text) {
      this.#drafts.delete(step.path);
    } else {
      this.#drafts.set(step.path, step.after);
    }
  }

  async undo(): Promise<void> {
    const step = this.#undo.pop();
    if (step) {
      this.#change({ path: step.path, after: step.before });
      this.#redo.push(step);
      await this.#play();
    }
  }

  async redo(): Promise<void> {
    const step = this.#redo.pop();
    if (step) {
      this.#change(step);
      this.#undo.push(step);
      await this.#play();
    }
  }

  /** Writes the drafts: sources first, the timeline last, each over the file as it was read.
   * A file changed outside keeps its draft and is said to conflict. */
  async save(): Promise<void> {
    const paths = [...this.#drafts.keys()].sort(
      (a, b) => Number(a === this.#entry) - Number(b === this.#entry),
    );
    for (const path of paths) {
      const text = this.#drafts.get(path)!;
      try {
        const sha256 = await service.write(path, text, this.#files.get(path)?.sha256 ?? null);
        this.#files.set(path, { text, sha256 });
        this.#drafts.delete(path);
        this.#conflicts.delete(path);
      } catch (error) {
        if (!(error instanceof Conflict)) {
          throw error;
        }
        this.#conflicts.add(path);
      }
    }
    this.#status = this.#conflicts.size ? "" : "saved";
    await this.#refresh();
  }

  /** Settles a conflict: the file as it is on disk, or the draft written over it. */
  async settle(path: string, keep: "theirs" | "mine"): Promise<void> {
    await this.#read(path);
    this.#conflicts.delete(path);
    if (keep === "theirs") {
      this.#drafts.delete(path);
      await this.#play();
    } else {
      await this.save();
    }
  }

  async #event(event: Record<string, any>): Promise<void> {
    if (event.event === "change") {
      const path = event.path as string;
      if (this.#drafts.has(path)) {
        this.#conflicts.add(path);
      } else if (this.#files.has(path)) {
        await this.#read(path);
        await this.#play();
      }
    } else if (event.event === "render") {
      const job = this.#jobs.get(event.id) ?? { id: event.id, done: 0, frames: 0 };
      if (event.progress) {
        [job.done, job.frames] = [event.progress.done, event.progress.frames];
      }
      if (event.result) {
        job.result = event.result;
        void this.#refresh();
      }
      this.#jobs.set(event.id, job);
    }
    this.requestUpdate();
  }

  /** Renders what is open natively, into `renders/`. */
  async export(): Promise<number> {
    const stem = this.#entry
      .split("/")
      .pop()!
      .replace(/(\.motion\.tsx|\.timeline\.json|\.json)$/, "");
    const out = `renders/${stem}.mp4`;
    const args =
      this.session!.kind === "project"
        ? ["project", "render", this.#entry]
        : this.timeline
          ? ["timeline", "render", this.#entry, "-o", out]
          : ["motion", "render", this.#entry, "-o", out];
    const size = this.renderRoot.querySelector<HTMLSelectElement>("#output-size")?.value;
    if (size) {
      const info = this.player?.info;
      const output = info && info.height > info.width ? size.split("x").reverse().join("x") : size;
      args.push("--output-size", output);
    }
    const id = await service.render(args);
    this.#jobs.set(id, { id, done: 0, frames: 0 });
    this.requestUpdate();
    return id;
  }

  /** The render of `id`, once the service says it is done. */
  job(id: number): Job | undefined {
    return this.#jobs.get(id);
  }

  get drafts(): string[] {
    return [...this.#drafts.keys()];
  }

  get conflicts(): string[] {
    return [...this.#conflicts];
  }

  #key = (event: KeyboardEvent) => {
    const typing = event.composedPath()[0] instanceof HTMLTextAreaElement;
    const mod = event.metaKey || event.ctrlKey;
    if (mod && event.key === "s") {
      event.preventDefault();
      void this.save();
    } else if (!typing && mod && event.key.toLowerCase() === "z") {
      event.preventDefault();
      void (event.shiftKey ? this.redo() : this.undo());
    } else if (!typing && (event.key === "Delete" || event.key === "Backspace") && this.#selected) {
      void this.edit({ op: "delete", clip: this.#selected });
      this.#selected = undefined;
    } else if (!typing && event.key === " ") {
      event.preventDefault();
      if (this.player?.playing) {
        this.player.pause();
      } else {
        this.player?.play();
      }
    }
  };

  updated(): void {
    const view = this.renderRoot.querySelector<TimelineView>("timeline-view");
    view?.set(this.layout, this.#selected, this.#time);
  }

  render() {
    const session = this.session;
    if (!session) {
      return html`<header><span class="name">Valle Studio</span></header>`;
    }
    if (session.kind === "home") {
      return this.#home(session);
    }
    return html`
      <header>
        <span class="name">${session.name} · ${this.#entry}</span>
        <span>${this.#status}</span>
        ${
          session.timelines && session.timelines.length > 1
            ? html`<select
                @change=${(e: Event) => void this.#load((e.target as HTMLSelectElement).value)}
              >
                ${session.timelines.map(
                  (t) => html`<option ?selected=${t === this.#entry}>${t}</option>`,
                )}
              </select>`
            : nothing
        }
        <button ?disabled=${!this.#undo.length} @click=${() => void this.undo()}>Undo</button>
        <button ?disabled=${!this.#redo.length} @click=${() => void this.redo()}>Redo</button>
        <button ?disabled=${!this.#drafts.size} @click=${() => void this.save()}>
          Save${this.#drafts.size ? ` (${this.#drafts.size})` : ""}
        </button>
        <select id="output-size" aria-label="Export resolution">
          <option value="">Design size</option>
          <option value="1280x720">720p</option>
          <option value="1920x1080">1080p</option>
          <option value="3840x2160">4K</option>
          <option value="7680x4320">8K</option>
        </select>
        <button @click=${() => void this.export()}>Export</button>
      </header>
      ${this.#left()}
      <section class="stage">
        <div class="canvas"></div>
        ${this.#error ? html`<div class="error">${this.#error}</div>` : nothing}
        ${this.#transport()}
      </section>
      ${
        this.timeline
          ? html`<section class="bottom">
              <timeline-view
                @select=${(e: CustomEvent) => {
                  this.#selected = e.detail;
                  this.requestUpdate();
                }}
                @op=${(e: CustomEvent) => void this.edit(e.detail)}
                @seek=${(e: CustomEvent) => this.player?.seek(e.detail)}
              ></timeline-view>
            </section>`
          : nothing
      }
      ${this.#right()}
    `;
  }

  #transport() {
    const player = this.player;
    const info = player?.info;
    return html`<div class="transport">
      <button
        ?disabled=${!player}
        @click=${() => (player?.playing ? player.pause() : player?.play())}
      >
        ${player?.playing ? "Pause" : "Play"}
      </button>
      <input
        type="range"
        min="0"
        max=${info ? info.frames - 1 : 0}
        .value=${String(player?.currentFrame ?? 0)}
        @input=${(e: Event) => player?.seekFrame(Number((e.target as HTMLInputElement).value))}
      />
      <span>${this.#time.toFixed(2)} / ${info?.duration.toFixed(2) ?? "–"} s</span>
      <label
        ><input
          type="checkbox"
          .checked=${player?.loop ?? false}
          @change=${(e: Event) => player && (player.loop = (e.target as HTMLInputElement).checked)}
        />Loop</label
      >
      <label
        ><input
          type="checkbox"
          .checked=${player?.muted ?? false}
          @change=${(e: Event) => player && (player.muted = (e.target as HTMLInputElement).checked)}
        />Mute</label
      >
    </div>`;
  }

  #left() {
    const renders = this.#listed.filter((f) => f.path.startsWith("renders/"));
    const files = this.#listed.filter((f) => !f.path.startsWith("renders/"));
    return html`<aside class="left">
      <h3>Files</h3>
      ${files.map(
        (f) =>
          html`<button
            class="file ${f.path === this.#open ? "open" : ""}"
            title=${f.path}
            ?disabled=${!TEXT.test(f.path)}
            @click=${() => void this.#show(f.path)}
          >
            ${this.#drafts.has(f.path) ? "● " : ""}${f.path} <small>${size(f.size)}</small>
          </button>`,
      )}
      ${
        this.session?.kind === "project"
          ? html`<h3>History</h3>
              <button @click=${() => void this.#snapshot()}>Save a snapshot</button>
              ${this.#history.map(
                (s) =>
                  html`<button class="file" @click=${() => void this.#restore(s.snapshot)}>
                    #${s.snapshot} ${s.message ?? ""}
                    <small>${new Date(s.time * 1000).toLocaleString()}</small>
                  </button>`,
              )}`
          : nothing
      }
      <h3>Renders</h3>
      ${[...this.#jobs.values()].map(
        (j) =>
          html`<div>
            #${j.id}
            ${
              j.result
                ? j.result.status === "ok"
                  ? "done"
                  : html`<span title=${j.result.message}>failed</span>`
                : `${j.done}/${j.frames || "…"}`
            }
          </div>`,
      )}
      ${renders.map(
        (f) =>
          html`<a class="file" href=${link(f.path)} target="_blank"
            >${f.path.slice(8)} <small>${size(f.size)}</small></a
          >`,
      )}
    </aside>`;
  }

  #right() {
    const conflicts = [...this.#conflicts];
    return html`<aside class="right">
      ${conflicts.map(
        (path) =>
          html`<div class="banner">
            ${path} changed on disk.
            <button @click=${() => void this.settle(path, "theirs")}>Take theirs</button>
            <button @click=${() => void this.settle(path, "mine")}>Keep mine</button>
          </div>`,
      )}
      ${this.#inspector()} ${this.#props()}
      <h3>Source · ${this.#open}</h3>
      <textarea
        spellcheck="false"
        wrap="off"
        .value=${this.text(this.#open)}
        @input=${(e: Event) => {
          this.#change({ path: this.#open, after: (e.target as HTMLTextAreaElement).value });
          this.requestUpdate();
          this.#later();
        }}
      ></textarea>
    </aside>`;
  }

  /** The selected clip, as JSON that is edited field by field. */
  #inspector() {
    const at = this.#selected;
    if (!this.timeline || !at) {
      return nothing;
    }
    const clip = find(JSON.parse(this.text(this.#entry)), at);
    if (!clip) {
      return nothing;
    }
    return html`<h3>Clip · ${at}</h3>
      <textarea
        wrap="off"
        style="min-height:120px;flex:0 0 auto"
        .value=${JSON.stringify(clip, null, 2)}
        @change=${(e: Event) => void this.#apply(at, clip, (e.target as HTMLTextAreaElement).value)}
      ></textarea>`;
  }

  /** Sets each field of the clip at `at` that `text` changes, as one step to undo. */
  async #apply(at: string, before: Record<string, unknown>, text: string): Promise<void> {
    let after: Record<string, unknown>;
    try {
      after = JSON.parse(text);
    } catch (error) {
      this.#status = (error as Error).message;
      this.requestUpdate();
      return;
    }
    const path = this.#entry;
    const start = this.text(path);
    let current = start;
    try {
      for (const field of new Set([...Object.keys(before), ...Object.keys(after)])) {
        if (JSON.stringify(before[field]) !== JSON.stringify(after[field])) {
          const op = { op: "set", clip: at, field, value: after[field] ?? null };
          ({ text: current } = await service.edit(current, op));
        }
      }
    } catch (error) {
      this.#status = (error as Error).message;
      this.requestUpdate();
      return;
    }
    this.#change({ path, after: current });
    this.#undo.push({ path, before: start, after: current });
    this.#redo = [];
    await this.#play();
  }

  /** A scene's props, as its controls declare them: values the preview is played with. */
  #props() {
    const props = this.player?.controls?.props ?? {};
    const names = Object.keys(props);
    if (this.timeline || names.length === 0) {
      return nothing;
    }
    const set = (name: string, value: unknown) => {
      const values = { ...this.#values, [name]: value };
      this.player!.setProps(values).then(
        () => {
          this.#values = values;
          this.#status = "";
        },
        (error: Error) => (this.#status = error.message),
      );
    };
    return html`<h3>Props</h3>
      ${names.map((name) => {
        const control = props[name];
        const value = this.#values[name] ?? control.default;
        const input = (type: string, read: (e: HTMLInputElement) => unknown) =>
          html`<input
            type=${type}
            .value=${String(value ?? "")}
            .checked=${value === true}
            min=${control.min ?? nothing}
            max=${control.max ?? nothing}
            step=${control.step ?? "any"}
            @change=${(e: Event) => set(name, read(e.target as HTMLInputElement))}
          />`;
        const field =
          control.type === "select"
            ? html`<select
                @change=${(e: Event) => set(name, (control.values ?? [])[Number((e.target as HTMLSelectElement).value)])}
              >
                ${(control.values ?? []).map(
                  (v, i) => html`<option value=${i} ?selected=${v === value}>${String(v)}</option>`,
                )}
              </select>`
            : control.type === "number"
              ? input("number", (e) => Number(e.value))
              : control.type === "boolean"
                ? input("checkbox", (e) => e.checked)
                : input("text", (e) => e.value);
        return html`<label class="field">${control.label ?? name} ${field}</label>`;
      })}`;
  }

  async #show(path: string): Promise<void> {
    if (!this.#files.has(path)) {
      await this.#read(path);
    }
    this.#open = path;
    this.requestUpdate();
  }

  async #snapshot(): Promise<void> {
    const message = prompt("What changed?") ?? "";
    await this.save();
    const done = await service.project(message ? ["save", "-m", message] : ["save"]);
    this.#status = done.summary ?? "";
    await this.#refresh();
  }

  async #restore(n: number): Promise<void> {
    if (!confirm(`Put snapshot ${n}'s sources back? What they replace is saved first.`)) {
      return;
    }
    const done = await service.project(["restore", String(n)]);
    this.#status = done.summary ?? done.message ?? "";
    this.#files.clear();
    this.#drafts.clear();
    await this.#load(this.#entry);
  }

  #home(session: Session) {
    const open = async (folder: string) => {
      try {
        this.session = await service.open(folder);
        await this.#load(this.session.entry!);
      } catch (error) {
        this.#status = (error as Error).message;
      }
      this.requestUpdate();
    };
    const create = async () => {
      const folder = prompt("Where? A folder to create the project in") ?? "";
      if (folder) {
        const done = await service.project(["create", folder]);
        if (done.status !== "ok") {
          this.#status = done.message;
          this.requestUpdate();
          return;
        }
        await open(done.root ?? folder);
      }
    };
    return html`<header><span class="name">Valle Studio</span><span>${this.#status}</span></header>
      <div class="home">
        <h3>Projects opened last</h3>
        ${(session.recent ?? []).map(
          (p) => html`<button class="file" @click=${() => void open(p.path)}>${p.path}</button>`,
        )}
        <h3>Open a folder</h3>
        <form
          @submit=${(e: Event) => {
            e.preventDefault();
            const input = (e.target as HTMLFormElement).elements.namedItem("folder");
            void open((input as HTMLInputElement).value);
          }}
        >
          <input name="folder" placeholder="/path/to/project" size="40" />
          <button>Open</button>
        </form>
        <h3>New</h3>
        <button @click=${() => void create()}>Create a project…</button>
      </div>`;
  }
}

/** The value at a document path such as `tracks.visual[0].items[2]`. */
function find(root: any, path: string): Record<string, unknown> | undefined {
  let here = root;
  for (const part of path.split(".")) {
    const [key, ...indices] = part.split("[");
    here = here?.[key];
    for (const index of indices) {
      here = here?.[Number(index.slice(0, -1))];
    }
  }
  return here;
}

function size(bytes: number): string {
  return bytes < 1024
    ? `${bytes} B`
    : bytes < 1 << 20
      ? `${(bytes / 1024).toFixed(0)} KB`
      : `${(bytes / (1 << 20)).toFixed(1)} MB`;
}

customElements.define("valle-studio", ValleStudio);
