// Studio: what `valle motion|timeline|project studio` opens, played by the SDK and edited in
// place. Unsaved changes are drafts the preview plays; saving writes the sources first, then the
// timeline, each over the file as it was read, so a change made outside is never lost. Exports
// are native renders the service runs.

import { html, LitElement, nothing } from "lit";
import { styles } from "./styles.ts";
import { icon } from "./icons.ts";
import { addMedia, find, selection } from "./edit.ts";
import { createPlayer, type Player, type Stats } from "../../../packages/engine/src/index.ts";

import {
  Conflict,
  fetching,
  type Layout,
  address,
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
  static styles = styles;

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
  /** Finish or reject a media edit before navigation or another mutation. */
  #editing: Promise<void> | undefined;
  #status = "";
  #time = 0;
  #shownFrame = 0;
  #scrub: { frame: number; dragging: boolean; resume: boolean } | undefined;
  #stats: Stats | undefined;
  /** Each opening of the player, so one superseded is let go. */
  #opening = 0;
  #reload: ReturnType<typeof setTimeout> | undefined;
  /** The props the preview of a scene is played with. */
  #values: Record<string, unknown> = {};
  #leftTab: "files" | "media" | "history" | "renders" = "files";
  #rightTab: "properties" | "code" = "properties";
  #filter = "";
  #asset: Listed | undefined;
  #focused = false;
  #inspectorVisible = innerWidth > 850;
  #collapsed = false;
  #zoom = 1;
  #busy = false;
  #exporting = false;
  #exportJob: number | undefined;
  #events: EventSource | undefined;

  connectedCallback(): void {
    super.connectedCallback();
    try {
      const prefs = JSON.parse(localStorage.getItem("valle.studio.workspace") ?? "{}");
      for (const area of ["left", "right", "bottom"] as const) {
        if (Number.isFinite(prefs[area])) this.#dimension(area, prefs[area]);
      }
    } catch {
      /* Preferences are optional. */
    }
    addEventListener("keydown", this.#key);
    void this.#run(() => this.#start());
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    removeEventListener("keydown", this.#key);
    this.#events?.close();
    clearTimeout(this.#reload);
    this.#opening++;
    void this.player?.close();
  }

  async #start(): Promise<void> {
    this.session = await service.session();
    this.#events = service.events((event) => void this.#run(() => this.#event(event)));
    if (this.session.kind !== "home") {
      await this.#load(this.session.entry!);
    }
    this.requestUpdate();
  }

  /** Opens `entry`: its text, the folder's files, and a player of it. */
  async #load(entry: string): Promise<void> {
    if (this.#editing) await this.#editing;
    const opening = ++this.#opening;
    const changed = entry !== this.#entry;
    const playing = this.player?.playing ?? false;
    const time = changed ? 0 : this.#busy ? this.#time : (this.player?.currentTime ?? 0);
    clearTimeout(this.#reload);
    this.player?.pause();
    if (changed) {
      this.#values = {};
      this.#exportJob = undefined;
    }
    this.#entry = this.#open = entry;
    this.#selected = undefined;
    this.#asset = undefined;
    this.layout = undefined;
    this.#time = time;
    this.#scrub = undefined;
    this.#error = this.#status = "";
    this.#busy = true;
    this.setAttribute("mode", this.timeline ? "timeline" : "motion");
    this.requestUpdate();
    try {
      // Keep a draft's original revision: rereading it here would hide an outside edit.
      if (!this.#files.has(entry)) await this.#read(entry);
      if (opening !== this.#opening) return;
      await this.#refresh();
      await this.updateComplete;
      if (opening === this.#opening) await this.#play(time, playing);
    } catch (error) {
      if (opening === this.#opening) {
        this.#busy = false;
        this.#error = (error as Error).message;
        this.requestUpdate();
      }
    }
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
  async #play(
    time = this.player?.currentTime ?? this.#time,
    playing = this.player?.playing ?? false,
  ): Promise<boolean> {
    const opening = ++this.#opening;
    this.#scrub = undefined;
    this.#busy = true;
    this.#stats = undefined;
    this.requestUpdate();
    const entry = this.#entry;
    const loop = this.player?.loop ?? false;
    const muted = this.player?.muted ?? false;
    const previous = this.player;
    previous?.pause();
    if (opening !== this.#opening) return false;
    this.#stats = undefined;
    const canvas = document.createElement("canvas");
    const stage = this.renderRoot.querySelector(".canvas");

    const files: Record<string, Blob> = {};
    for (const [path, text] of this.#drafts) {
      files[path] = new Blob([text]);
    }
    try {
      const layout = this.timeline ? (await service.edit(this.text(entry))).layout : undefined;
      if (opening !== this.#opening) return false;
      this.layout = layout;
      if (layout?.duration === 0) {
        this.player = undefined;
        await previous?.close();
        stage?.replaceChildren();
        this.#error = "";
        this.#busy = false;
        this.requestUpdate();
        return true;
      }
      const player = await createPlayer({
        canvas,
        runtime: "/valle/",
        entry,
        base: "/api/files/",
        fonts: "/api/fonts/",
        fetch: fetching,
        files,
        props: this.#values,
        loop,
        muted,
      });
      if (opening !== this.#opening) {
        await player.close();
        return false;
      }
      this.player = player;
      stage?.replaceChildren(canvas);
      await previous?.close();
      this.#shownFrame = player.currentFrame;
      this.#error = "";
      player.on("time", ({ time, frame }) => {
        if (this.player !== player) return;
        this.#time = time;
        this.#shownFrame = frame;
        this.#finishScrub();
        this.requestUpdate();
      });
      player.on("error", (error) => {
        if (this.player !== player) return;
        this.#error = error.message;
        this.requestUpdate();
      });
      player.on("stats", (stats) => {
        if (this.player !== player) return;
        this.#stats = stats;
        this.requestUpdate();
      });
      player.on("play", () => {
        if (this.player !== player) return;
        this.#stats = undefined;
        this.requestUpdate();
      });
      for (const event of ["pause", "ended"] as const) {
        player.on(event, () => this.requestUpdate());
      }
      player.seek(time);
      if (playing) player.play();
    } catch (error) {
      if (opening === this.#opening) {
        this.#error = (error as Error).message;
      }
    }
    if (opening === this.#opening) this.#busy = false;
    this.requestUpdate();
    return opening === this.#opening && !this.#error;
  }

  /** Plays the drafts once typing stops. */
  #later(): void {
    clearTimeout(this.#reload);
    this.#reload = setTimeout(() => void this.#play(), 400);
  }

  /** Does `op` to the timeline open: a draft of it, which can be undone. */
  async edit(op: unknown): Promise<void> {
    return this.#mutate(async () => {
      const path = this.#entry;
      const before = this.text(path);
      try {
        const { text, layout } = await service.edit(before, op);
        const selected = selection(this.layout, layout, this.#selected, op);
        await this.#accept({ path, before, after: text }, layout, selected);
      } catch (error) {
        this.#status = (error as Error).message;
        this.requestUpdate();
      }
    });
  }

  /** Serialize mutations through validation; switching files and saving wait for them too. */
  async #mutate(action: () => Promise<void>): Promise<void> {
    const pending = (this.#editing ?? Promise.resolve()).then(action, action);
    this.#editing = pending;
    try {
      await pending;
    } finally {
      if (this.#editing === pending) this.#editing = undefined;
    }
  }

  /** Keep the last valid preview and reject an edit that cannot be played. */
  async #accept(step: Step, layout: Layout, selected?: string): Promise<void> {
    const previous = { layout: this.layout, selected: this.#selected };
    this.#change(step);
    this.layout = layout;
    this.#selected = selected;
    if (await this.#play()) {
      this.#undo.push(step);
      this.#redo = [];
    } else {
      this.#change({ path: step.path, after: step.before });
      this.layout = previous.layout;
      this.#selected = previous.selected;
      this.#status = `Edit not applied: ${this.#error}`;
      this.#error = "";
    }
    this.requestUpdate();
  }

  #change(step: { path: string; after: string }): void {
    this.#status = "";
    if (step.after === this.#files.get(step.path)?.text) {
      this.#drafts.delete(step.path);
    } else {
      this.#drafts.set(step.path, step.after);
    }
  }

  async undo(): Promise<void> {
    if (this.#editing) await this.#editing;
    const step = this.#undo.pop();
    if (step) {
      this.#change({ path: step.path, after: step.before });
      this.#redo.push(step);
      await this.#load(step.path);
      this.#status = `Undid edit in ${step.path}`;
      this.requestUpdate();
    }
  }

  async redo(): Promise<void> {
    if (this.#editing) await this.#editing;
    const step = this.#redo.pop();
    if (step) {
      this.#change(step);
      this.#undo.push(step);
      await this.#load(step.path);
      this.#status = `Redid edit in ${step.path}`;
      this.requestUpdate();
    }
  }

  /** Writes the drafts: sources first, the timeline last, each over the file as it was read.
   * A file changed outside keeps its draft and is said to conflict. */
  async save(): Promise<void> {
    if (this.#editing) await this.#editing;
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
    if (this.#editing) await this.#editing;
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

  /** The system save panel chooses the destination, then Rust renders directly to it. */
  async export(): Promise<number | undefined> {
    const name =
      this.#entry
        .split("/")
        .pop()!
        .replace(/(\.motion\.tsx|\.tsx|\.timeline\.json|\.json)$/, "") + ".mp4";
    const out = await service.saveFile(name);
    if (!out) {
      this.#status = "Export cancelled.";
      this.requestUpdate();
      return undefined;
    }
    await this.save();
    if (this.#conflicts.size) throw new Error("Resolve the file conflict before exporting.");
    const group =
      this.session!.kind === "project" && this.timeline
        ? "project"
        : this.timeline
          ? "timeline"
          : "motion";
    const args = [group, "render", this.#entry, "-o", out];
    const size = this.renderRoot.querySelector<HTMLSelectElement>("#output-size")?.value;
    if (size) {
      const info = this.player?.info;
      const output = info && info.height > info.width ? size.split("x").reverse().join("x") : size;
      args.push("--output-size", output);
    }
    // The system save panel already confirmed replacement of an existing file.
    const id = await service.render(args, true);
    if (!this.#jobs.has(id)) this.#jobs.set(id, { id, done: 0, frames: 0 });
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
    if (this.renderRoot.querySelector("dialog[open]")) return;
    const target = event.composedPath()[0];
    const typing =
      target instanceof HTMLElement &&
      (target.matches("input, textarea, select") || target.isContentEditable);
    const mod = event.metaKey || event.ctrlKey;
    if (mod && event.key === "s") {
      event.preventDefault();
      void this.#run(() => this.save());
    } else if (!typing && mod && event.key.toLowerCase() === "z") {
      event.preventDefault();
      void (event.shiftKey ? this.redo() : this.undo());
    } else if (!typing && (event.key === "Delete" || event.key === "Backspace") && this.#selected) {
      event.preventDefault();
      void this.edit({ op: "delete", clip: this.#selected });
    } else if (
      !typing &&
      !(target instanceof HTMLElement && target.closest("button, a, [role=button]")) &&
      event.key === " "
    ) {
      event.preventDefault();
      if (this.player?.playing) {
        this.player.pause();
      } else {
        this.player?.play();
      }
    }
  };

  updated(): void {
    this.toggleAttribute("focused", this.#focused);
    this.toggleAttribute("inspector-hidden", !this.#inspectorVisible);
    this.toggleAttribute("collapsed", this.timeline && this.#collapsed);
    const view = this.renderRoot.querySelector<TimelineView>("timeline-view");
    view?.set(this.layout, this.#selected, this.#time, this.#zoom);
  }

  async #run(action: () => Promise<unknown>): Promise<void> {
    try {
      await action();
    } catch (error) {
      this.#status = (error as Error).message;
    }
    this.requestUpdate();
  }

  #dimension(area: "left" | "right" | "bottom", value: number): void {
    const [min, max] =
      area === "bottom"
        ? [120, Math.max(120, innerHeight - 280)]
        : area === "left"
          ? [180, 360]
          : [260, 560];
    this.style.setProperty(`--${area}`, `${Math.min(max, Math.max(min, value))}px`);
  }

  #remember(): void {
    const prefs = Object.fromEntries(
      ["left", "right", "bottom"].map((area) => [
        area,
        parseFloat(getComputedStyle(this).getPropertyValue(`--${area}`)),
      ]),
    );
    try {
      localStorage.setItem("valle.studio.workspace", JSON.stringify(prefs));
    } catch {
      /* Preferences are optional. */
    }
  }

  #resize(event: PointerEvent, area: "left" | "right" | "bottom"): void {
    if (event.button !== 0) return;
    event.preventDefault();
    const handle = event.currentTarget as HTMLElement;
    const initial = parseFloat(getComputedStyle(this).getPropertyValue(`--${area}`));
    const move = (e: PointerEvent) => {
      const by =
        area === "bottom"
          ? event.clientY - e.clientY
          : (e.clientX - event.clientX) * (area === "left" ? 1 : -1);
      this.#dimension(area, initial + by);
    };
    const stop = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", stop);
      handle.removeEventListener("pointercancel", stop);
      this.#remember();
    };
    handle.setPointerCapture(event.pointerId);
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", stop);
    handle.addEventListener("pointercancel", stop);
  }

  #separator(area: "left" | "right" | "bottom") {
    return html`<div
      class=${area === "bottom" ? "horizontal" : `split ${area}-split`}
      role="separator"
      aria-label=${`Resize ${area === "bottom" ? "timeline" : area + " panel"}`}
      aria-orientation=${area === "bottom" ? "horizontal" : "vertical"}
      tabindex="0"
      @pointerdown=${(e: PointerEvent) => this.#resize(e, area)}
      @keydown=${(e: KeyboardEvent) => {
        const keys =
          area === "bottom"
            ? ["ArrowDown", "ArrowUp"]
            : area === "right"
              ? ["ArrowRight", "ArrowLeft"]
              : ["ArrowLeft", "ArrowRight"];
        if (!keys.includes(e.key)) return;
        e.preventDefault();
        this.#dimension(
          area,
          parseFloat(getComputedStyle(this).getPropertyValue(`--${area}`)) +
            (keys.indexOf(e.key) ? 20 : -20),
        );
        this.#remember();
      }}
    ></div>`;
  }

  render() {
    const session = this.session;
    if (!session)
      return html`${this.#header()}
        <div class="home muted">${this.#status || "Opening Studio…"}</div>`;
    if (session.kind === "home") return this.#home(session);
    const info = this.player?.info;
    return html`
      ${this.#header()}
      <main class="workspace">
        ${this.#left()} ${this.#separator("left")}
        <section class="stage panel" aria-label="Preview">
          <div class="panel-head preview-head">
            <strong>Preview</strong>
            ${this.#playbackRate()}
            <span class="preview-meta">
              <small>${info ? `${info.width} × ${info.height}` : ""}</small>
              <span class="badge">Fit</span>
            </span>
          </div>
          <div class="canvas"></div>
          ${this.layout?.duration === 0 ? html`<div class="empty">Choose a file in Media, then add it to the timeline.</div>` : nothing}
          ${
            this.#error
              ? html`<div class="error" role="alert">
                  <pre>${this.#error}</pre>
                  <button @click=${() => this.#run(() => this.#play())}>Retry preview</button>
                </div>`
              : nothing
          }
          ${this.#transport()}
        </section>
        ${this.#separator("right")} ${this.#right()}
      </main>
      ${this.#separator("bottom")}
      <section class="bottom panel" aria-label="Timeline" ?hidden=${!this.timeline}>
        <div class="panel-head timeline-tools">
          <strong>Timeline</strong>
          <small class="hint"
            >${this.layout?.tracks.length ?? 0} tracks · Drag to move, edges to trim</small
          >
          <span class="spacer"></span>
          <button
            class="quiet"
            @click=${() => {
              this.#zoom = 1;
              this.requestUpdate();
            }}
          >
            Fit
          </button>
          <input
            type="range"
            aria-label="Timeline zoom"
            min="1"
            max="10"
            step="0.25"
            .value=${String(this.#zoom)}
            @input=${(e: Event) => {
              this.#zoom = Number((e.target as HTMLInputElement).value);
              this.requestUpdate();
            }}
          />
          <button
            class="icon quiet"
            aria-label=${this.#collapsed ? "Expand timeline" : "Collapse timeline"}
            @click=${() => {
              this.#collapsed = !this.#collapsed;
              this.requestUpdate();
            }}
          >
            ${icon("chevron")}
          </button>
        </div>
        <timeline-view
          ?hidden=${this.#collapsed}
          @select=${(e: CustomEvent) => {
            this.#selected = e.detail;
            this.#asset = undefined;
            this.#rightTab = "properties";
            this.#inspectorVisible = true;
            this.requestUpdate();
          }}
          @op=${(e: CustomEvent) => this.#run(() => this.edit(e.detail))}
          @seek=${(e: CustomEvent) => this.player?.seek(e.detail)}
        ></timeline-view>
      </section>
      <footer class="statusbar" role="status">
        <span class="message">${this.#status || (this.#busy ? "Updating preview…" : "Ready")}</span
        ><span class="spacer"></span><span>${info ? `${info.frames} frames` : ""}</span
        ><span>Local workspace</span>
      </footer>
      ${this.#exportDialog()}
    `;
  }

  #header() {
    const session = this.session;
    const dirty = this.#drafts.size;
    return html`<header class="topbar">
      <div class="brand">
        <img class="logo" src="/brand/valle-mark.svg" alt="Valle" />
      </div>
      ${
        session && session.kind !== "home"
          ? html`
              <span class="divider"></span
              ><span class="name" title=${this.#entry}
                >${session.kind === "project" ? session.name : this.#entry.split("/").pop()}</span
              >
              <span class="badge">${this.timeline ? "timeline" : "motion"}</span>
              ${
                this.timeline && session.timelines && session.timelines.length > 1
                  ? html`<select
                      aria-label="Timeline"
                      @change=${(e: Event) => this.#run(() => this.#load((e.target as HTMLSelectElement).value))}
                    >
                      ${session.timelines.map((t) => html`<option ?selected=${t === this.#entry}>${t}</option>`)}
                    </select>`
                  : nothing
              }
              <span class="spacer"></span>
              <span class="save-state ${dirty ? "dirty" : ""}"
                >${dirty ? html`<span class="dot">●</span>${dirty} unsaved` : html`${icon("check")}Saved`}</span
              >
              <button
                class="icon quiet"
                title="Undo (⌘/Ctrl Z)"
                aria-label="Undo"
                ?disabled=${!this.#undo.length}
                @click=${() => this.#run(() => this.undo())}
              >
                ${icon("undo")}
              </button>
              <button
                class="icon quiet"
                title="Redo (⇧⌘/Ctrl Z)"
                aria-label="Redo"
                ?disabled=${!this.#redo.length}
                @click=${() => this.#run(() => this.redo())}
              >
                ${icon("redo")}
              </button>
              <button
                class="icon quiet"
                title="Save (⌘/Ctrl S)"
                aria-label="Save"
                ?disabled=${!dirty}
                @click=${() => this.#run(() => this.save())}
              >
                ${icon("save")}
              </button>
              <span class="divider"></span>
              <button
                class="icon quiet"
                title="Focus preview"
                aria-label="Focus preview"
                aria-pressed=${this.#focused}
                @click=${() => {
                  this.#focused = !this.#focused;
                  this.requestUpdate();
                }}
              >
                ${icon("focus")}
              </button>
              <button
                class="icon quiet"
                title="Inspector"
                aria-label="Toggle inspector"
                aria-pressed=${this.#inspectorVisible && !this.#focused}
                @click=${() => {
                  this.#focused = false;
                  this.#inspectorVisible = !this.#inspectorVisible;
                  this.requestUpdate();
                }}
              >
                ${icon("panel")}
              </button>
              <button
                class="primary"
                ?disabled=${this.#busy || !this.player || !!this.#error}
                @click=${async () => {
                  await this.updateComplete;
                  this.renderRoot.querySelector<HTMLDialogElement>("#export-dialog")?.showModal();
                }}
              >
                ${icon("export")}Export
              </button>
            `
          : html`<span class="spacer"></span
              ><span class="muted">Your local creative workspace</span>`
      }
    </header>`;
  }

  #playbackRate() {
    const fps = !this.#busy && this.player?.playing ? this.#stats?.fps : undefined;
    return html`<span
      class="playback-rate"
      data-metric="fps"
      aria-label="Live playback FPS"
      title="Actual playback frame rate, updated about once a second."
      >${fps === undefined ? "—" : fps.toFixed(1)} fps</span
    >`;
  }

  /** Hold playback while dragging; resume after the final requested frame is actually drawn. */
  #seekPreview(frame: number): void {
    const player = this.player;
    if (!player) return;
    this.#scrub ??= { frame, dragging: false, resume: player.playing };
    this.#scrub.frame = frame;
    player.pause();
    player.seekFrame(frame);
    this.#finishScrub();
    this.requestUpdate();
  }

  #finishScrub(): void {
    const scrub = this.#scrub;
    if (!scrub || scrub.dragging || scrub.frame !== this.#shownFrame) return;
    this.#scrub = undefined;
    if (scrub.resume) this.player?.play();
  }

  #transport() {
    const player = this.player;
    const info = player?.info;
    const frame = this.#scrub?.frame ?? this.#shownFrame;
    const [n, d = 1] = (info?.fps ?? "30").split("/").map(Number);
    const seconds = this.#scrub ? (frame * d) / n : this.#time;
    const progress = info && info.frames > 1 ? frame / (info.frames - 1) : 0;
    const release = () => {
      if (this.#scrub) this.#scrub.dragging = false;
      this.#finishScrub();
      this.requestUpdate();
    };
    return html`<div class="transport">
      <div
        class="scrubber"
        data-playing=${!!player?.playing && !this.#scrub && frame > 0}
        data-disabled=${!player}
        style=${`--scrub-progress: ${progress}; --scrub-step: ${Math.min(100, (1000 * d) / n)}ms`}
      >
        <div class="scrub-rail" aria-hidden="true">
          <div class="scrub-fill"></div>
          <div class="scrub-thumb"></div>
        </div>
        <input
          class="scrub"
          type="range"
          aria-label="Preview frame"
          aria-valuetext=${`${clock(seconds)}, frame ${frame}`}
          min="0"
          max=${info ? info.frames - 1 : 0}
          step="1"
          .value=${String(frame)}
          ?disabled=${!player}
          @pointerdown=${(e: PointerEvent) => {
            if (!player || e.button !== 0) return;
            this.#scrub = { frame, dragging: true, resume: player.playing };
            player.pause();
            player.seekFrame(frame);
            (e.currentTarget as HTMLInputElement).setPointerCapture(e.pointerId);
            this.requestUpdate();
          }}
          @input=${(e: Event) => this.#seekPreview(Number((e.target as HTMLInputElement).value))}
          @pointerup=${release}
          @pointercancel=${release}
          @lostpointercapture=${release}
        />
      </div>
      <span class="time"
        >${clock(seconds)} <span class="muted">/ ${clock(info?.duration ?? 0)}</span></span
      >
      <span class="playback" role="group" aria-label="Playback controls">
        <button
          class="icon quiet"
          title="Previous frame"
          aria-label="Previous frame"
          ?disabled=${!player}
          @click=${() => player?.seekFrame(player.currentFrame - 1)}
        >
          ${icon("back")}
        </button>
        <button
          class="icon"
          title="Play / pause (Space)"
          aria-label=${player?.playing ? "Pause" : "Play"}
          ?disabled=${!player}
          @click=${() => (player?.playing ? player.pause() : player?.play())}
        >
          ${icon(player?.playing ? "pause" : "play")}
        </button>
        <button
          class="icon quiet"
          title="Next frame"
          aria-label="Next frame"
          ?disabled=${!player}
          @click=${() => player?.seekFrame(player.currentFrame + 1)}
        >
          ${icon("next")}
        </button>
      </span>
      <span class="toggle-group">
        <button
          class="icon quiet"
          title="Loop"
          aria-label="Loop"
          aria-pressed=${player?.loop ?? false}
          ?disabled=${!player}
          @click=${() => {
            if (player) player.loop = !player.loop;
            this.requestUpdate();
          }}
        >
          ${icon("loop")}
        </button>
        <button
          class="icon quiet"
          title="Mute"
          aria-label="Mute"
          aria-pressed=${player?.muted ?? false}
          ?disabled=${!player}
          @click=${() => {
            if (player) player.muted = !player.muted;
            this.requestUpdate();
          }}
        >
          ${icon(player?.muted ? "muted" : "volume")}
        </button>
      </span>
    </div>`;
  }

  #left() {
    const files = this.#listed.filter(
      (f) =>
        !f.path.startsWith("renders/") && f.path.toLowerCase().includes(this.#filter.toLowerCase()),
    );
    const media = files.filter((f) => mediaKind(f.path) !== "file");
    const groups = new Map<string, Listed[]>();
    for (const file of files) {
      const folder = file.path.includes("/") ? file.path.slice(0, file.path.lastIndexOf("/")) : "";
      groups.set(folder, [...(groups.get(folder) ?? []), file]);
    }
    return html`<aside class="left panel" aria-label="Project browser">
      <div class="tabs" role="tablist" aria-label="Project panels">
        ${(
          [
            ["files", "Files"],
            ["media", "Media"],
            ["history", "History"],
            ["renders", "Renders"],
          ] as const
        ).map(
          ([tab, name]) =>
            html`<button
              role="tab"
              aria-selected=${this.#leftTab === tab}
              @click=${() => {
                this.#leftTab = tab;
                this.requestUpdate();
              }}
            >
              ${name}
            </button>`,
        )}
      </div>
      ${
        this.#leftTab === "files" || this.#leftTab === "media"
          ? html`
              <label class="search"
                >${icon("search")}<input
                  aria-label="Filter files"
                  placeholder="Search files…"
                  .value=${this.#filter}
                  @input=${(e: Event) => {
                    this.#filter = (e.target as HTMLInputElement).value;
                    this.requestUpdate();
                  }}
              /></label>
              ${
                this.#leftTab === "files"
                  ? html`<div class="scroll files">
                      ${
                        groups.size
                          ? [...groups].map(
                              ([folder, entries]) =>
                                html`<details open>
                                  <summary>${folder || "Project files"}</summary>
                                  ${entries.map(
                                    (f) =>
                                      html`<button
                                        class="file ${f.path === this.#open ? "open" : ""}"
                                        title=${f.path}
                                        @click=${() => this.#openFile(f)}
                                      >
                                        ${icon(TEXT.test(f.path) ? "code" : mediaKind(f.path))}<span
                                          class="filename"
                                          >${f.path.split("/").pop()}</span
                                        >${this.#drafts.has(f.path) ? html`<span class="dot">●</span>` : nothing}<small
                                          >${size(f.size)}</small
                                        >
                                      </button>`,
                                  )}
                                </details>`,
                            )
                          : html`<div class="empty">No matching files.</div>`
                      }
                    </div>`
                  : html`<div class="scroll" style="padding:0">
                      ${
                        media.length
                          ? html`<div class="media-grid">
                              ${media.map(
                                (f) =>
                                  html`<button
                                    class="asset"
                                    title=${f.path}
                                    @click=${() => this.#openFile(f)}
                                  >
                                    <span class="thumbnail"
                                      >${mediaKind(f.path) === "image" ? html`<img loading="lazy" src=${address(f.path)} alt="" />` : icon(mediaKind(f.path))}</span
                                    ><span>${f.path.split("/").pop()}</span>
                                  </button>`,
                              )}
                            </div>`
                          : html`<div class="empty">
                              ${icon("image")}<strong>No media yet</strong>Images, video and audio
                              in this folder appear here.
                            </div>`
                      }
                    </div>`
              }
            `
          : this.#leftTab === "history"
            ? html`<div class="scroll">
                ${
                  this.session?.kind === "project"
                    ? html`<button @click=${() => this.#run(() => this.#snapshot())}>
                          ${icon("history")}Save a snapshot
                        </button>
                        ${this.#history.map(
                          (s) =>
                            html`<button
                              class="file"
                              @click=${() => this.#run(() => this.#restore(s.snapshot))}
                            >
                              ${icon("history")}<span class="filename"
                                >#${s.snapshot} ${s.message || "Snapshot"}<br /><small
                                  >${new Date(s.time).toLocaleString()}</small
                                ></span
                              >
                            </button>`,
                        )}
                        ${!this.#history.length ? html`<div class="empty">Save a snapshot to keep a version of your project.</div>` : nothing}`
                    : html`<div class="empty">
                        ${icon("history")}<strong>Project history</strong>Open a project to save and
                        restore snapshots.
                      </div>`
                }
              </div>`
            : html`<div class="scroll">
                ${[...this.#jobs.values()].reverse().map((job) => this.#renderJob(job))}
                ${this.#listed.filter((f) => f.path.startsWith("renders/")).map((f) => html`<a class="file" href=${address(f.path)} target="_blank" rel="noopener">${icon("film")}<span class="filename">${f.path.slice(8)}</span><small>${size(f.size)}</small></a>`)}
                ${!this.#jobs.size && !this.#listed.some((f) => f.path.startsWith("renders/")) ? html`<div class="empty">${icon("export")}<strong>Your finished work</strong>Export a video to see it here.</div>` : nothing}
              </div>`
      }
    </aside>`;
  }

  #openFile(file: Listed): void {
    this.#focused = false;
    this.#inspectorVisible = true;
    const preview =
      /\.tsx$|\.timeline\.json$|(?:^|\/)timeline\.json$/.test(file.path) ||
      file.path === this.session?.entry ||
      this.session?.timelines?.includes(file.path);
    if (preview) {
      this.#rightTab = "properties";
      if (file.path !== this.#entry) void this.#run(() => this.#load(file.path));
      else {
        this.#open = file.path;
        this.#asset = undefined;
      }
    } else if (TEXT.test(file.path)) void this.#run(() => this.#show(file.path));
    else {
      this.#asset = file;
      this.#rightTab = "properties";
    }
    this.requestUpdate();
  }

  #right() {
    return html`<aside class="right panel" aria-label="Inspector">
      <div class="tabs" role="tablist" aria-label="Inspector panels">
        ${(
          [
            ["properties", "Properties"],
            ["code", "Code"],
          ] as const
        ).map(
          ([tab, name]) =>
            html`<button
              role="tab"
              aria-selected=${this.#rightTab === tab}
              @click=${() => {
                this.#rightTab = tab;
                this.requestUpdate();
              }}
            >
              ${name}
            </button>`,
        )}
      </div>
      ${[...this.#conflicts].map(
        (path) =>
          html`<div class="banner" role="alert">
            <strong>File changed outside Studio</strong>
            <p>${path}</p>
            <div class="actions">
              <button @click=${() => this.#run(() => this.settle(path, "theirs"))}>
                Reload file</button
              ><button @click=${() => this.#run(() => this.settle(path, "mine"))}>
                Save my draft
              </button>
            </div>
          </div>`,
      )}
      ${this.#rightTab === "code" ? this.#source() : html`<div class="scroll">${this.#asset ? this.#media() : html`${this.#inspector()}${this.#props()}${this.#composition()}`}</div>`}
    </aside>`;
  }

  #source() {
    const source = this.text(this.#open);
    return html`<div class="source">
      <div class="source-name" title=${this.#open}>
        ${this.#open}${this.#drafts.has(this.#open) ? " ●" : ""}
      </div>
      <div class="editor">
        <pre class="gutter" aria-hidden="true">
${Array.from({ length: source.split("\n").length }, (_, i) => i + 1).join("\n")}</pre>
        <textarea
          aria-label="Source code"
          spellcheck="false"
          wrap="off"
          .value=${source}
          @scroll=${(e: Event) => {
            const input = e.target as HTMLTextAreaElement;
            const gutter = input.previousElementSibling!;
            gutter.scrollTop = input.scrollTop;
          }}
          @input=${(e: Event) => {
            this.#change({ path: this.#open, after: (e.target as HTMLTextAreaElement).value });
            this.requestUpdate();
            this.#later();
          }}
          @keydown=${(e: KeyboardEvent) => {
            if (e.key === "Tab" && !e.shiftKey && !e.metaKey && !e.ctrlKey) {
              e.preventDefault();
              const input = e.target as HTMLTextAreaElement;
              input.setRangeText("  ", input.selectionStart, input.selectionEnd, "end");
              input.dispatchEvent(new Event("input"));
            }
          }}
        ></textarea>
      </div>
      <div class="source-foot">
        <span>${source.split("\n").length} lines · UTF-8</span><span>⌘/Ctrl S to save</span>
      </div>
    </div>`;
  }

  #composition() {
    const info = this.player?.info;
    return html`<section class="section">
      <h3>Composition</h3>
      ${
        info
          ? html`<div class="field">Canvas<span>${info.width} × ${info.height}</span></div>
              <div class="field">Frame rate<span>${info.fps} fps</span></div>
              <div class="field">
                Duration<span>${clock(info.duration)} · ${info.frames} frames</span>
              </div>`
          : html`<p class="muted">
              ${this.#busy ? "Loading composition…" : "Preview unavailable"}
            </p>`
      }
      ${this.timeline && !this.#selected ? html`<div class="empty">Select a clip on the timeline to edit its properties.</div>` : nothing}
    </section>`;
  }

  #media() {
    const file = this.#asset!;
    const url = address(file.path);
    const kind = mediaKind(file.path);
    return html`<section class="section">
      <h3>Media</h3>
      ${kind === "image" ? html`<img class="media-preview" src=${url} alt=${file.path} />` : kind === "film" ? html`<video class="media-preview" src=${url} controls preload="metadata"></video>` : kind === "music" ? html`<audio class="media-preview" src=${url} controls preload="metadata"></audio>` : nothing}
      <p style="overflow-wrap:anywhere">${file.path}</p>
      <div class="field">Size<span>${size(file.size)}</span></div>
      <a href=${url} target="_blank" rel="noopener">Open file ↗</a>
      ${this.timeline && kind !== "file" ? html`<p><button class="primary" ?disabled=${this.#busy} @click=${() => this.#run(() => this.#addMedia(file))}>Add to timeline</button></p>` : nothing}
    </section>`;
  }

  async #addMedia(file: Listed): Promise<void> {
    return this.#mutate(async () => {
      const kind = mediaKind(file.path);
      let duration = 3;
      if (kind === "film" || kind === "music") {
        const media = this.renderRoot.querySelector<HTMLMediaElement>(".media-preview")!;
        if (media.readyState < 1) {
          await new Promise<void>((resolve, reject) => {
            const finish = (error?: Error) => {
              clearTimeout(timer);
              media.removeEventListener("loadedmetadata", loaded);
              media.removeEventListener("error", failed);
              if (error) reject(error);
              else resolve();
            };
            const loaded = () => finish();
            const failed = () => finish(new Error(`Cannot read ${file.path}`));
            const timer = setTimeout(
              () => finish(new Error(`Timed out reading ${file.path}`)),
              10000,
            );
            media.addEventListener("loadedmetadata", loaded);
            media.addEventListener("error", failed);
          });
        }
        duration = media.duration;
        if (!Number.isFinite(duration) || duration <= 0)
          throw new Error("The media has no finite duration.");
      }
      const path = this.#entry;
      const before = this.text(path);
      const added = addMedia(
        before,
        path,
        file.path,
        kind === "film" ? "video" : kind === "music" ? "audio" : "image",
        duration,
      );
      const { text, layout } = await service.edit(added.text);
      const at = layout.tracks.flatMap((t) => t.clips).find((c) => c.id === added.id)?.at;
      await this.#accept({ path, before, after: text }, layout, at);
      this.#asset = undefined;
      this.requestUpdate();
    });
  }

  #inspector() {
    const at = this.#selected;
    if (!this.timeline || !at) return nothing;
    let clip: Record<string, any> | undefined;
    try {
      clip = find(JSON.parse(this.text(this.#entry)), at);
    } catch {
      return nothing;
    }
    if (!clip) return nothing;
    const placed = this.layout?.tracks.flatMap((t) => t.clips).find((c) => c.at === at);
    const set = (field: string, value: unknown) =>
      this.#run(() => this.edit({ op: "set", clip: at, field, value }));
    return html`<section class="section">
      <h3>Selected clip · ${clip.kind ?? placed?.kind ?? ""}</h3>
      <label class="field"
        >Name<input
          aria-label="Clip name"
          .value=${String(clip.name ?? "")}
          placeholder=${placed?.id ?? "Clip"}
          @change=${(e: Event) => set("name", (e.target as HTMLInputElement).value)}
      /></label>
      ${at.includes(".clips[") ? html`<label class="field">Start (s)<input type="number" min="0" step="0.01" .value=${String(placed?.start ?? 0)} @change=${(e: Event) => this.#run(() => this.edit({ op: "move", clip: at, start: Number((e.target as HTMLInputElement).value) }))} /></label>` : nothing}
      <label class="field"
        >Duration (s)<input
          type="number"
          min="0.01"
          step="0.01"
          .value=${String(placed?.duration ?? 0)}
          @change=${(e: Event) => this.#run(() => this.edit({ op: "trim", clip: at, duration: Number((e.target as HTMLInputElement).value) }))}
      /></label>
      <label class="field"
        >Enabled<input
          type="checkbox"
          .checked=${clip.enabled !== false}
          @change=${(e: Event) => set("enabled", (e.target as HTMLInputElement).checked)}
      /></label>
      ${clip.kind === "solid" ? html`<label class="field">Color<input aria-label="Clip color" .value=${String(clip.color ?? "")} @change=${(e: Event) => set("color", (e.target as HTMLInputElement).value)} /></label>` : nothing}
      <details>
        <summary>Advanced · clip JSON</summary>
        <textarea
          aria-label="Clip JSON"
          spellcheck="false"
          wrap="off"
          .value=${JSON.stringify(clip, null, 2)}
          @change=${(e: Event) => this.#run(() => this.#apply(at, clip!, (e.target as HTMLTextAreaElement).value))}
        ></textarea>
      </details>
    </section>`;
  }

  #renderJob(job: Job) {
    const result = job.result;
    const state = result
      ? result.status === "ok"
        ? "complete"
        : "error"
      : !job.frames
        ? "preparing"
        : job.done >= job.frames
          ? "finishing"
          : "rendering";
    const label = {
      preparing: "Preparing export",
      rendering: "Rendering video",
      finishing: "Finishing file",
      complete: "Export complete",
      error: "Export failed",
    }[state];
    // The last frame still has to leave the encoder and the file has to close successfully.
    const percent =
      state === "complete"
        ? 100
        : job.frames
          ? Math.min(99, Math.floor((100 * job.done) / job.frames))
          : 0;
    const frames = result?.frames ?? job.frames;
    return html`<div class="render-card" data-state=${state}>
      <div class="render-heading">
        <span class="render-label">
          <span class="render-mark" aria-hidden="true"
            >${state === "complete" ? icon("check") : state === "error" ? icon("close") : nothing}</span
          >
          <strong role="status">${label}</strong>
        </span>
        <span class="render-percent"
          >${state === "preparing" || (state === "error" && !job.frames) ? "—" : `${percent}%`}</span
        >
      </div>
      <div
        class="render-track"
        role="progressbar"
        aria-label="Export progress"
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow=${state === "preparing" || (state === "error" && !job.frames) ? nothing : percent}
        aria-valuetext=${state === "rendering" ? `${percent}%, ${job.done} of ${job.frames} frames` : label}
      >
        <div class="render-fill" style=${`--render-progress: ${percent / 100}`}></div>
      </div>
      <div class="render-detail">
        <span
          >${state === "preparing" ? "Preparing scene and encoder…" : state === "finishing" ? "Finalizing video…" : state === "complete" ? `${frames} frames` : state === "error" ? "Export stopped" : `${job.done.toLocaleString()} / ${frames.toLocaleString()} frames`}</span
        >
        ${state === "complete" && Number.isFinite(result?.seconds) ? html`<span>${result.seconds.toFixed(1)} s</span>` : nothing}
      </div>
      ${result ? html`<p class="render-path" title=${result.message || result.file || result.summary}>${result.message || result.file || result.summary}</p>` : nothing}
      ${state === "complete" && result?.url ? html`<a class="render-open" href=${result.url} target="_blank" rel="noopener">Open video ↗</a>` : nothing}
    </div>`;
  }

  #exportDialog() {
    const job = this.#exportJob === undefined ? undefined : this.#jobs.get(this.#exportJob);
    const running = this.#exporting || (job !== undefined && !job.result);
    return html`<dialog id="export-dialog" aria-labelledby="export-heading">
      <form
        @submit=${(e: Event) => {
          e.preventDefault();
          void this.#startExport();
        }}
      >
        <div class="dialog-body">
          <div class="actions">
            <h2 id="export-heading">Export video</h2>
            <span class="spacer"></span
            ><button
              type="button"
              class="icon quiet"
              aria-label="Close export"
              @click=${() => this.renderRoot.querySelector<HTMLDialogElement>("#export-dialog")?.close()}
            >
              ${icon("close")}
            </button>
          </div>
          <p>${this.#entry.split("/").pop()} · MP4</p>
          <label class="field"
            >Resolution<select id="output-size" aria-label="Export resolution" ?disabled=${running}>
              <option value="">Design size</option>
              <option value="1280x720">720p</option>
              <option value="1920x1080">1080p</option>
              <option value="3840x2160">4K</option>
              <option value="7680x4320">8K</option>
            </select></label
          >
          <p>Choose where to save in the system dialog.</p>
          ${this.#drafts.size ? html`<p>Unsaved changes will be saved before export.</p>` : nothing}
          ${job ? this.#renderJob(job) : nothing}
          ${this.#status && !job ? html`<p role="status">${this.#status}</p>` : nothing}
        </div>
        <div class="dialog-foot">
          <button
            type="button"
            @click=${() => this.renderRoot.querySelector<HTMLDialogElement>("#export-dialog")?.close()}
          >
            Close</button
          ><button class="primary" type="submit" ?disabled=${running}>
            ${icon("export")}${running ? "Exporting…" : job ? "Export again" : "Export video"}
          </button>
        </div>
      </form>
    </dialog>`;
  }

  async #startExport(): Promise<void> {
    if (this.#exporting) return;
    this.#exporting = true;
    this.#exportJob = undefined;
    this.#status = "";
    this.requestUpdate();
    await this.#run(async () => {
      this.#exportJob = await this.export();
    });
    this.#exporting = false;
    this.requestUpdate();
  }

  /** Sets each field of the clip at `at` that `text` changes, as one step to undo. */
  async #apply(at: string, before: Record<string, unknown>, text: string): Promise<void> {
    return this.#mutate(async () => {
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
      const { layout } = await service.edit(current);
      await this.#accept(
        { path, before: start, after: current },
        layout,
        selection(
          this.layout,
          layout,
          at,
          before.id !== after.id
            ? { op: "set", clip: at, field: "id", value: after.id }
            : undefined,
        ),
      );
    });
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
      this.player!.setProps(values)
        .then(
          () => {
            this.#values = values;
            this.#status = "";
          },
          (error: Error) => (this.#status = error.message),
        )
        .finally(() => this.requestUpdate());
    };
    return html`<section class="section">
      <h3>Preview properties</h3>
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
      })}
    </section>`;
  }

  async #show(path: string): Promise<void> {
    if (!this.#files.has(path)) {
      await this.#read(path);
    }
    this.#open = path;
    this.#asset = undefined;
    this.#rightTab = "code";
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
    return html`${this.#header()}
      <div class="home">
        <h1>Make something move.</h1>
        <p>Open a project to bring your scenes, media and timeline together.</p>
        <p role="status">${this.#status}</p>
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

function clock(seconds: number): string {
  const m = Math.floor(seconds / 60);
  return `${String(m).padStart(2, "0")}:${(seconds % 60).toFixed(2).padStart(5, "0")}`;
}

function mediaKind(path: string): "image" | "film" | "music" | "file" {
  if (/\.(png|jpe?g|webp|gif|svg|avif)$/i.test(path)) return "image";
  if (/\.(mp4|mov|webm|mkv)$/i.test(path)) return "film";
  if (/\.(wav|mp3|m4a|aac|ogg|flac)$/i.test(path)) return "music";
  return "file";
}

function size(bytes: number): string {
  return bytes < 1024
    ? `${bytes} B`
    : bytes < 1 << 20
      ? `${(bytes / 1024).toFixed(0)} KB`
      : `${(bytes / (1 << 20)).toFixed(1)} MB`;
}

customElements.define("valle-studio", ValleStudio);
