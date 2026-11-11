// Studio: what `valle motion|timeline|project studio` opens, played by the SDK and edited in
// place. Unsaved changes are drafts the preview plays; saving writes the sources first, then the
// timeline, each over the file as it was read, so a change made outside is never lost. Exports
// are native renders the service runs.

import { html, LitElement, nothing } from "lit";
import { styles } from "./styles.ts";
import { icon } from "./icons.ts";
import { addMedia, find, selection } from "./edit.ts";
import { frame, invalidate, PEAKS, peaks, wave } from "./media.ts";
import { createPlayer, type Player, type Stats } from "../../../packages/engine/src/index.ts";

import {
  Conflict,
  fetching,
  type Layout,
  address,
  mediaUrl,
  type Listed,
  type Session,
  service,
} from "./service.ts";
import { type TimelineView, title, ZOOM } from "./timeline-view.ts";

/** Files the source panel edits as text. */
const TEXT = /\.(tsx?|json|css|txt|md|srt|vtt|lrc|csv|wgsl)$/;

/** An edit undone and redone: a file's text before and after. */
type Step = { path: string; before: string; after: string };

/** A render the service runs: how far it is, and what it made. */
type Job = { id: number; done: number; frames: number; result?: Record<string, any> };

/** A question asked in a dialog: a snapshot's message, a restore, a new project's place. */
type Ask =
  { kind: "snapshot" } | { kind: "restore"; snapshot: number } | { kind: "create"; folder: string };

const BLENDS =
  "normal multiply screen overlay darken lighten color-dodge color-burn hard-light soft-light difference exclusion hue saturation color luminosity plus-lighter".split(
    " ",
  );

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
  /** Whether dragged edges snap to the playhead, markers and other clips. */
  #snap = true;
  /** A refusal said over the workspace until it fades. */
  #toast = "";
  #fading: ReturnType<typeof setTimeout> | undefined;
  #ask: Ask | undefined;

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
    invalidate();
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
    return this.#entry.endsWith(".timeline.json");
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
        const { text, layout, summary } = await service.edit(before, op);
        const selected = selection(this.layout, layout, this.#selected, op);
        await this.#accept({ path, before, after: text }, layout, selected);
        if (this.text(path) === text) {
          this.#status = summary;
          this.requestUpdate();
        }
      } catch (error) {
        this.#refuse(this.#explain((error as Error).message));
      }
    });
  }

  /** Says why an edit was not made: in the status bar, and over the workspace for a while. */
  #refuse(message: string): void {
    this.#status = this.#toast = message;
    clearTimeout(this.#fading);
    this.#fading = setTimeout(() => {
      this.#toast = "";
      this.requestUpdate();
    }, 6000);
    this.requestUpdate();
  }

  /** The service's message with clips called by their names, not their paths. */
  #explain(message: string): string {
    const clips = this.layout?.tracks.flatMap((t) => t.clips) ?? [];
    return message.replace(/tracks\.\w+\[\d+\]\.(?:items|clips)\[\d+\]/g, (at) => {
      const clip = clips.find((c) => c.at === at);
      return clip ? `“${title(clip)}”` : at;
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
    const view = this.renderRoot.querySelector<TimelineView>("timeline-view");
    const before = JSON.parse(step.before),
      after = JSON.parse(step.after);
    for (const track of this.layout?.tracks ?? []) {
      if (!view?.locked(track.id)) continue;
      const next = layout.tracks.find((t) => t.id === track.id);
      if (
        JSON.stringify(find(before, track.at)) !== JSON.stringify(next && find(after, next.at)) ||
        JSON.stringify(track.clips) !== JSON.stringify(next?.clips)
      ) {
        this.#refuse(`Track “${track.id}” is locked. Unlock it before editing.`);
        return;
      }
    }
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
      this.#refuse(`Edit not applied: ${this.#explain(this.#error)}`);
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
      invalidate(mediaUrl("", path));
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
        .replace(/(\.motion\.tsx|\.timeline\.json)$/, "") + ".mp4";
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
    } else if (
      !typing &&
      !mod &&
      !event.altKey &&
      !(target instanceof HTMLElement && target.closest("[role=separator]"))
    ) {
      this.#transportKey(event);
    }
  };

  /** ← → and J step a frame, a second with Shift; Home and End go to the ends; K pauses and L
   * plays; Escape lets the selection go. */
  #transportKey(event: KeyboardEvent): void {
    const player = this.player;
    const last = (player?.info.frames ?? 1) - 1;
    const step = (frames: number) => {
      player?.pause();
      player?.seekFrame(Math.min(last, Math.max(0, player.currentFrame + frames)));
    };
    const by = event.shiftKey ? Math.round(this.#rate) : 1;
    const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
    if (key === "Escape" && this.#selected) {
      this.#selected = undefined;
      this.requestUpdate();
      return;
    }
    if (!player) return;
    if (key === "ArrowLeft" || key === "j") step(-by);
    else if (key === "ArrowRight") step(by);
    else if (key === "Home") step(-last);
    else if (key === "End") step(last);
    else if (key === "k") player.pause();
    else if (key === "l") player.play();
    else return;
    event.preventDefault();
  }

  /** Frames a second, as a number. */
  get #rate(): number {
    const [n, d = 1] = (this.player?.info.fps ?? "30").split("/").map(Number);
    return n / d;
  }

  updated(): void {
    this.toggleAttribute("focused", this.#focused);
    this.toggleAttribute("inspector-hidden", !this.#inspectorVisible);
    this.toggleAttribute("collapsed", this.timeline && this.#collapsed);
    const view = this.renderRoot.querySelector<TimelineView>("timeline-view");
    view?.set({
      layout: this.layout,
      selected: this.#selected,
      // The frame a scrub asks for, before the player has drawn it.
      time: this.#scrub ? this.#scrub.frame / this.#rate : this.#time,
      zoom: this.#zoom,
      entry: this.#entry,
      scope: `${this.session?.root ?? ""}/${this.#entry}`,
      playing: !!this.player?.playing && !this.#scrub,
      snap: this.#snap,
    });
    const ask = this.renderRoot.querySelector<HTMLDialogElement>("#ask");
    if (ask && !ask.open) ask.showModal();
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
          <span class="spacer"></span>
          <button
            class="icon quiet"
            title="Snap to the playhead, markers and clips (hold Alt to drag freely)"
            aria-label="Snapping"
            aria-pressed=${this.#snap}
            @click=${() => {
              this.#snap = !this.#snap;
              this.requestUpdate();
            }}
          >
            ${icon("magnet")}
          </button>
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
            max=${ZOOM}
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
            if (e.detail) {
              this.#asset = undefined;
              this.#rightTab = "properties";
              // A narrow window keeps its room for the preview unless the inspector is asked for.
              if (innerWidth > 850) this.#inspectorVisible = true;
            }
            this.requestUpdate();
          }}
          @op=${(e: CustomEvent) => this.#run(() => this.edit(e.detail))}
          @refuse=${(e: CustomEvent) => this.#refuse(e.detail)}
          @zoom=${(e: CustomEvent) => {
            this.#zoom = e.detail;
            this.requestUpdate();
          }}
          @seek=${(e: CustomEvent) => this.#seekTimeline(e.detail.time, e.detail.phase !== "end")}
        ></timeline-view>
      </section>
      <footer class="statusbar" role="status">
        <span class="message" title=${this.#status}
          >${this.#status || (this.#busy ? "Updating preview…" : "Ready")}</span
        ><span class="spacer"></span
        ><span
          >${info ? `${info.width} × ${info.height} · ${info.fps} fps · ${info.frames} frames` : ""}</span
        >
      </footer>
      ${this.#toast ? html`<div class="toast" role="alert">${this.#toast}</div>` : nothing}
      ${this.#exportDialog()}${this.#askDialog()}
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
                class="icon quiet"
                title="Export video"
                aria-label="Export"
                ?disabled=${this.#busy || !this.player || !!this.#error}
                @click=${async () => {
                  await this.updateComplete;
                  this.renderRoot.querySelector<HTMLDialogElement>("#export-dialog")?.showModal();
                }}
              >
                ${icon("export")}
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

  /** The timeline's ruler dragged to `seconds`; `dragging` until it is let go. */
  #seekTimeline(seconds: number, dragging: boolean): void {
    const player = this.player;
    if (!player) return;
    const frame = Math.min(player.info.frames - 1, Math.round(seconds * this.#rate));
    this.#scrub ??= { frame, dragging, resume: player.playing };
    this.#scrub.frame = frame;
    this.#scrub.dragging = dragging;
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
    const rate = n / d;
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
          aria-valuetext=${`${timecode(frame, rate)}, frame ${frame}`}
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
      <span class="time" title="Hours : minutes : seconds : frames"
        >${timecode(frame, rate)} <span class="muted">/ ${timecode(info?.frames ?? 0, rate)}</span
        ><span class="frame">${frame} f</span></span
      >
      <span class="playback" role="group" aria-label="Playback controls">
        <button
          class="icon quiet"
          title="Previous frame (←)"
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
          title="Next frame (→)"
          aria-label="Next frame"
          ?disabled=${!player}
          @click=${() => player?.seekFrame(player.currentFrame + 1)}
        >
          ${icon("next")}
        </button>
      </span>
      <span class="toggle-group">
        ${this.#playbackRate()}
        <small class="canvas-size">${info ? `${info.width} × ${info.height}` : ""}</small>
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
                                        class="file ${f.path === (this.#asset?.path ?? this.#open) ? "open" : ""}"
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
                                    class="asset ${f.path === this.#asset?.path ? "selected" : ""}"
                                    title=${f.path}
                                    aria-pressed=${f.path === this.#asset?.path}
                                    @click=${() => this.#openFile(f)}
                                  >
                                    <span class="thumbnail">${this.#thumbnail(f)}</span
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
                    ? html`<button
                          class="file"
                          @click=${() => {
                            this.#ask = { kind: "snapshot" };
                            this.requestUpdate();
                          }}
                        >
                          ${icon("history")}Save a snapshot
                        </button>
                        ${this.#history.map(
                          (s) =>
                            html`<button
                              class="file"
                              @click=${() => {
                                this.#ask = { kind: "restore", snapshot: s.snapshot };
                                this.requestUpdate();
                              }}
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

  /** A media file's picture: the image, a video's frame, a sound's shape. */
  #thumbnail(file: Listed) {
    const kind = mediaKind(file.path);
    const url = address(file.path);
    const redraw = () => this.requestUpdate();
    if (kind === "image") return html`<img loading="lazy" src=${url} alt="" />`;
    const picture = kind === "film" ? frame(url, 1, redraw) : undefined;
    if (picture) return html`<img src=${picture} alt="" />`;
    const sound = kind === "music" ? peaks(url, redraw) : undefined;
    if (sound) {
      const shape = wave(sound, 0, sound.length / PEAKS);
      return html`<svg class="wave" viewBox="0 0 ${shape.columns} 2" preserveAspectRatio="none">
        <path d=${shape.d}></path>
      </svg>`;
    }
    return icon(kind);
  }

  #openFile(file: Listed): void {
    this.#focused = false;
    this.#inspectorVisible = true;
    const preview = /\.motion\.tsx$|\.timeline\.json$/.test(file.path);
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
                Duration<span>${timecode(info.frames, this.#rate)} · ${info.frames} frames</span>
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
      ${kind === "image" ? html`<img class="media-preview" src=${url} alt=${file.path} />` : kind === "file" ? nothing : this.#mediaPlayer(url, kind)}
      <p style="overflow-wrap:anywhere">${file.path}</p>
      <div class="field">Size<span>${size(file.size)}</span></div>
      <div class="actions">
        ${this.timeline && kind !== "file" ? html`<button ?disabled=${this.#busy} @click=${() => this.#run(() => this.#addMedia(file))}>${icon("plus", 14)}Add to timeline</button>` : nothing}
        <a class="button" href=${url} target="_blank" rel="noopener">Open file ↗</a>
      </div>
    </section>`;
  }

  /** A video or a sound played in the inspector, with Studio's own controls: a sound is its
   * shape, clicked to go to a time. */
  #mediaPlayer(url: string, kind: "film" | "music") {
    const find = () =>
      this.renderRoot.querySelector<HTMLMediaElement>(":is(video, audio).media-preview");
    // The element as it was drawn last; its events ask for the next drawing.
    const media = find();
    const loaded = media?.getAttribute("src") === url;
    const duration = loaded && Number.isFinite(media!.duration) ? media!.duration : 0;
    const time = loaded ? media!.currentTime : 0;
    const playing = loaded && !media!.paused && !media!.ended;
    const progress = duration ? Math.min(1, time / duration) : 0;
    const redraw = () => this.requestUpdate();
    const follow = (e: Event) => {
      const playing = e.target as HTMLMediaElement;
      const tick = () => {
        this.requestUpdate();
        if (!playing.paused && !playing.ended && playing.isConnected) requestAnimationFrame(tick);
      };
      tick();
    };
    const toggle = () => {
      const media = find();
      if (media?.paused) void media.play();
      else media?.pause();
    };
    const seek = (seconds: number) => {
      const media = find();
      if (media) media.currentTime = seconds;
      this.requestUpdate();
    };
    const sound = kind === "music" ? peaks(url, redraw) : undefined;
    const shape = sound && wave(sound, 0, sound.length / PEAKS);
    const drawn = (played: boolean) =>
      html`<svg
        class=${played ? "played" : ""}
        style=${played ? `clip-path: inset(0 ${100 - progress * 100}% 0 0)` : ""}
        viewBox="0 0 ${shape!.columns} 2"
        preserveAspectRatio="none"
      >
        <path d=${shape!.d}></path>
      </svg>`;
    return html`<div class="media-player">
      ${
        kind === "film"
          ? html`<video
              class="media-preview"
              src=${url}
              preload="metadata"
              playsinline
              @click=${toggle}
              @play=${follow}
              @pause=${redraw}
              @ended=${redraw}
              @seeked=${redraw}
              @emptied=${redraw}
              @loadedmetadata=${redraw}
              @volumechange=${redraw}
            ></video>`
          : html`<audio
                class="media-preview"
                src=${url}
                preload="metadata"
                @play=${follow}
                @pause=${redraw}
                @ended=${redraw}
                @seeked=${redraw}
                @emptied=${redraw}
                @loadedmetadata=${redraw}
                @volumechange=${redraw}
              ></audio>
              <div
                class="media-wave"
                @click=${(e: MouseEvent) => {
                  const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
                  seek(((e.clientX - box.left) / box.width) * duration);
                }}
              >
                ${shape ? html`${drawn(false)}${drawn(true)}` : icon("music", 24)}
              </div>`
      }
      <div class="media-controls">
        <button
          class="icon"
          title=${playing ? "Pause" : "Play"}
          aria-label=${playing ? "Pause media" : "Play media"}
          @click=${toggle}
        >
          ${icon(playing ? "pause" : "play")}
        </button>
        <div class="scrubber" style=${`--scrub-progress: ${progress}`}>
          <div class="scrub-rail" aria-hidden="true">
            <div class="scrub-fill"></div>
            <div class="scrub-thumb"></div>
          </div>
          <input
            class="scrub"
            type="range"
            aria-label="Media position"
            min="0"
            max=${duration}
            step="0.01"
            .value=${String(time)}
            ?disabled=${!duration}
            @input=${(e: Event) => seek(Number((e.target as HTMLInputElement).value))}
          />
        </div>
        <span class="time">${span(time)} <span class="muted">/ ${span(duration)}</span></span>
        <button
          class="icon quiet"
          title="Mute"
          aria-label="Mute media"
          aria-pressed=${loaded && media!.muted}
          @click=${() => {
            const media = find();
            if (media) media.muted = !media.muted;
          }}
        >
          ${icon(loaded && media!.muted ? "muted" : "volume")}
        </button>
      </div>
    </div>`;
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
    const track = this.layout?.tracks.find((t) => t.clips.some((c) => c.at === at));
    const placed = track?.clips.find((c) => c.at === at);
    const kind = placed?.kind ?? "";
    const set = (field: string, value: unknown) =>
      this.#run(() => this.edit({ op: "set", clip: at, field, value }));
    // Keyframes are edited as JSON; a field shows that it has them.
    const keyed = (value: unknown) =>
      typeof value === "object" && value !== null && !Array.isArray(value);
    const number = (label: string, field: string, unset: number, step: number, range = "") => {
      const [min, max] = range.split(" ");
      return html`<label class="field"
        >${label}${
          keyed(clip[field])
            ? html`<span>Keyframes</span>`
            : html`<input
                type="number"
                aria-label=${label}
                step=${step}
                min=${min || nothing}
                max=${max || nothing}
                .value=${String(clip[field] ?? "")}
                placeholder=${String(unset)}
                @change=${(e: Event) => {
                  const text = (e.target as HTMLInputElement).value;
                  void set(field, text === "" ? null : Number(text));
                }}
              />`
        }</label
      >`;
    };
    const pair = (label: string, field: string, unset: [number, number], step: number) => {
      const value = clip[field];
      if (keyed(value)) return html`<div class="field">${label}<span>Keyframes</span></div>`;
      const change = (e: Event) => {
        const inputs = (e.target as HTMLElement).parentElement!.querySelectorAll("input");
        const texts = [...inputs].map((input) => input.value);
        const both = texts.map((text, i) => (text === "" ? unset[i] : Number(text)));
        void set(field, texts.every((text) => text === "") ? null : both);
      };
      return html`<div class="field">
        ${label}<span class="pair"
          >${[0, 1].map(
            (i) =>
              html`<input
                type="number"
                aria-label=${`${label} ${"XY"[i]}`}
                step=${step}
                .value=${String(value?.[i] ?? "")}
                placeholder=${String(unset[i])}
                @change=${change}
              />`,
          )}</span
        >
      </div>`;
    };
    // The first of `values` is what the field is without being written.
    const choice = (label: string, field: string, values: string[]) =>
      html`<label class="field"
        >${label}<select
          aria-label=${label}
          @change=${(e: Event) => {
            const select = e.target as HTMLSelectElement;
            void set(field, select.selectedIndex === 0 ? null : select.value);
          }}
        >
          ${values.map((v) => html`<option ?selected=${(clip[field] ?? values[0]) === v}>${v}</option>`)}
        </select></label
      >`;
    const [width, height] = [this.layout?.width ?? 0, this.layout?.height ?? 0];
    const sourced = ["video", "audio", "timeline", "motion"].includes(kind) && !clip.timeMap;
    const props = kind === "motion" && keyed(clip.props) ? Object.entries(clip.props) : [];
    const prop = (name: string, value: unknown) => {
      const give = (next: unknown) => void set("props", { ...clip.props, [name]: next });
      const input = (type: string, read: (e: HTMLInputElement) => unknown) =>
        html`<input
          type=${type}
          aria-label=${name}
          step="any"
          .value=${String(value)}
          .checked=${value === true}
          @change=${(e: Event) => give(read(e.target as HTMLInputElement))}
        />`;
      const field =
        typeof value === "number"
          ? input("number", (e) => Number(e.value))
          : typeof value === "boolean"
            ? input("checkbox", (e) => e.checked)
            : typeof value === "string"
              ? input("text", (e) => e.value)
              : html`<span
                  >${keyed(value) && "keyframes" in (value as object) ? "Keyframes" : "JSON"}</span
                >`;
      return html`<label class="field">${name} ${field}</label>`;
    };
    return html`<section class="section">
        <h3>Selected clip · ${clip.kind ?? kind}</h3>
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
        ${kind === "solid" ? html`<label class="field">Color<input aria-label="Clip color" .value=${String(clip.color ?? "")} @change=${(e: Event) => set("color", (e.target as HTMLInputElement).value)} /></label>` : nothing}
        ${
          kind === "caption" && typeof clip.text === "string"
            ? html`<label class="field stacked"
                >Text<textarea
                  aria-label="Caption text"
                  rows="3"
                  .value=${clip.text}
                  @change=${(e: Event) => set("text", (e.target as HTMLTextAreaElement).value)}
                ></textarea>
              </label>`
            : nothing
        }
        ${kind === "grade" ? number("Temperature", "temperature", 0, 0.05, "-1 1") : nothing}
      </section>
      ${
        track?.kind === "visual"
          ? html`<section class="section">
              <h3>Transform</h3>
              ${pair("Position", "position", [width / 2, height / 2], 1)}
              ${pair("Size", "size", [width, height], 1)} ${pair("Scale", "scale", [1, 1], 0.01)}
              ${number("Rotation (°)", "rotation", 0, 1)}
              ${number("Opacity", "opacity", 1, 0.05, "0 1")}
              ${choice("Fit", "fit", ["contain", "cover", "fill", "none"])}
              ${choice("Blend", "blend", BLENDS)}
            </section>`
          : nothing
      }
      ${
        sourced
          ? html`<section class="section">
              <h3>Source</h3>
              ${number("Trim start (s)", "trimStart", 0, 0.01, "0")}
              ${number("Speed", "rate", 1, 0.05, "0.01")}
              ${choice("Past its end", "end", ["error", "hold", "loop"])}
            </section>`
          : nothing
      }
      ${
        ["video", "audio", "timeline"].includes(kind)
          ? html`<section class="section">
              <h3>Sound</h3>
              ${number("Volume", "gain", 1, 0.05, "0")} ${number("Pan", "pan", 0, 0.05, "-1 1")}
            </section>`
          : nothing
      }
      ${
        props.length
          ? html`<section class="section">
              <h3>Scene properties</h3>
              ${props.map(([name, value]) => prop(name, value))}
            </section>`
          : nothing
      }
      <section class="section">
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

  /** The dialog `#ask` is asked in; what it is answered with is done. */
  #askDialog() {
    const ask = this.#ask;
    if (!ask) return nothing;
    const close = () => {
      this.#ask = undefined;
      this.requestUpdate();
    };
    const [heading, action] = {
      snapshot: ["Save a snapshot", "Save snapshot"],
      restore: ["Restore a snapshot", "Restore"],
      create: ["New project", "Create project"],
    }[ask.kind];
    return html`<dialog id="ask" aria-labelledby="ask-heading" @close=${close}>
      <form
        @submit=${(e: Event) => {
          e.preventDefault();
          const answer = new FormData(e.target as HTMLFormElement);
          close();
          void this.#run(() => this.#answer(ask, answer));
        }}
      >
        <div class="dialog-body">
          <h2 id="ask-heading">${heading}</h2>
          ${
            ask.kind === "snapshot"
              ? html`<p>
                    Keeps the project's sources as they are now. Unsaved changes are saved first.
                  </p>
                  <label class="field stacked"
                    >What changed?<input name="message" placeholder="Optional" autofocus
                  /></label>`
              : ask.kind === "restore"
                ? html`<p>
                    Puts snapshot #${ask.snapshot}'s sources back. What they replace is saved as a
                    snapshot first, and unsaved changes here are dropped.
                  </p>`
                : html`<p>A folder with a project.json, a first timeline and its media folders.</p>
                    <label class="field stacked"
                      >Name<input name="name" value="my-film" required autofocus
                    /></label>
                    <label class="field stacked"
                      >Location
                      <span class="actions"
                        ><input
                          name="folder"
                          placeholder="/path/to/folder"
                          required
                          .value=${ask.folder}
                          @input=${(e: Event) => (ask.folder = (e.target as HTMLInputElement).value)}
                        /><button
                          type="button"
                          @click=${() =>
                            this.#run(async () => {
                              ask.folder = (await service.chooseFolder()) ?? ask.folder;
                            })}
                        >
                          Choose…
                        </button></span
                      ></label
                    >`
          }
        </div>
        <div class="dialog-foot">
          <button type="button" @click=${close}>Cancel</button
          ><button class="primary" type="submit">${action}</button>
        </div>
      </form>
    </dialog>`;
  }

  async #answer(ask: Ask, answer: FormData): Promise<void> {
    if (ask.kind === "snapshot") {
      const message = String(answer.get("message") ?? "").trim();
      await this.save();
      const done = await service.project(message ? ["save", "-m", message] : ["save"]);
      this.#status = done.summary ?? "";
      await this.#refresh();
    } else if (ask.kind === "restore") {
      const done = await service.project(["restore", String(ask.snapshot)]);
      this.#status = done.summary ?? done.message ?? "";
      this.#files.clear();
      this.#drafts.clear();
      invalidate();
      await this.#load(this.#entry);
    } else {
      const folder = String(answer.get("folder")).replace(/[\\/]+$/, "");
      const done = await service.project(["create", `${folder}/${answer.get("name")}`]);
      if (done.status !== "ok") throw new Error(done.message);
      await this.#openProject(done.root ?? `${folder}/${answer.get("name")}`);
    }
  }

  async #openProject(folder: string): Promise<void> {
    this.session = await service.open(folder);
    invalidate();
    this.#files.clear();
    this.#drafts.clear();
    this.#conflicts.clear();
    this.#undo = [];
    this.#redo = [];
    this.#entry = "";
    this.#status = "";
    await this.#load(this.session.entry!);
  }

  #home(session: Session) {
    const recent = session.recent ?? [];
    return html`${this.#header()}
      <div class="home">
        <div class="hero">
          <img src="/brand/valle-mark.svg" alt="" />
          <div>
            <h1>Make something move.</h1>
            <p>Open a project to bring your scenes, media and timeline together.</p>
          </div>
        </div>
        <div class="home-actions">
          <button
            class="primary"
            @click=${() => {
              this.#ask = { kind: "create", folder: "" };
              this.requestUpdate();
            }}
          >
            ${icon("plus")}New project</button
          ><button
            @click=${() =>
              this.#run(async () => {
                const folder = await service.chooseFolder();
                if (folder) await this.#openProject(folder);
              })}
          >
            ${icon("folder")}Open folder…
          </button>
        </div>
        ${this.#status ? html`<p class="notice" role="status">${this.#status}</p>` : nothing}
        <h3>Recent projects</h3>
        ${
          recent.length
            ? html`<div class="projects">
                ${recent.map(
                  (p) =>
                    html`<button
                      class="project"
                      title=${p.path}
                      @click=${() => this.#run(() => this.#openProject(p.path))}
                    >
                      <span class="project-mark">${icon("film", 20)}</span
                      ><span class="project-text"
                        ><strong>${p.name}</strong><small>${p.path}</small></span
                      ><small>${ago(p.opened)}</small>
                    </button>`,
                )}
              </div>`
            : html`<div class="empty">
                ${icon("folder")}<strong>No projects yet</strong>Create one, or open a folder that
                has a project.json.
              </div>`
        }
        <details class="by-path">
          <summary>Open by path</summary>
          <form
            @submit=${(e: Event) => {
              e.preventDefault();
              const input = (e.target as HTMLFormElement).elements.namedItem("folder");
              void this.#run(() => this.#openProject((input as HTMLInputElement).value));
            }}
          >
            <input name="folder" aria-label="Project folder" placeholder="/path/to/project" />
            <button>Open</button>
          </form>
        </details>
      </div>
      ${this.#askDialog()}`;
  }
}

/** `frame` as a timecode, hours:minutes:seconds:frames, at `fps` frames a second. */
function timecode(frame: number, fps: number): string {
  const base = Math.max(1, Math.round(fps));
  const seconds = Math.floor(frame / base);
  const parts = [Math.floor(seconds / 3600), Math.floor(seconds / 60) % 60, seconds % 60];
  return [...parts, frame % base].map((n) => String(n).padStart(2, "0")).join(":");
}

/** How long ago `opened`, seconds since 1970, was. */
function ago(opened: number): string {
  const minutes = Math.floor((Date.now() / 1000 - opened) / 60);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 60 * 24) return `${Math.floor(minutes / 60)} h ago`;
  if (minutes < 60 * 24 * 30) return `${Math.floor(minutes / 60 / 24)} d ago`;
  return new Date(opened * 1000).toLocaleDateString();
}

function mediaKind(path: string): "image" | "film" | "music" | "file" {
  if (/\.(png|jpe?g|webp|gif|svg|avif)$/i.test(path)) return "image";
  if (/\.(mp4|mov|webm|mkv)$/i.test(path)) return "film";
  if (/\.(wav|mp3|m4a|aac|ogg|flac)$/i.test(path)) return "music";
  return "file";
}

/** A length of media as minutes:seconds, with tenths: 1:05.3. */
function span(seconds: number): string {
  const rest = (seconds % 60).toFixed(1).padStart(4, "0");
  return `${Math.floor(seconds / 60)}:${rest}`;
}

function size(bytes: number): string {
  return bytes < 1024
    ? `${bytes} B`
    : bytes < 1 << 20
      ? `${(bytes / 1024).toFixed(0)} KB`
      : `${(bytes / (1 << 20)).toFixed(1)} MB`;
}

customElements.define("valle-studio", ValleStudio);
