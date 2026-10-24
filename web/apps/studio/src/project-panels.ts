import { html, render } from "lit";
import type { StudioHost } from "./host.ts";
import type { ValleStudioApp } from "./studio-shell.ts";
import { StudioServices, requireImported, type AssetItem, type ImportIntent } from "./project-client.ts";

type Panel = "assets" | "history" | "exports";
const kinds = ["video", "audio", "image", "font", "model3d", "environment", "shader", "component", "data", "other"];
const fileUrl = (scope: "project" | "library", path: string) =>
  `${scope === "project" ? "/project/file/" : "/library/blob/"}${path.split("/").map(encodeURIComponent).join("/")}`;

export class ProjectPanels {
  readonly #services: StudioServices;
  readonly #panel = document.createElement("aside");
  #tab: Panel = "assets";
  #scope: "project" | "library";
  #query = "";
  #kind = "";
  #directory = "";
  #tag = "";
  #items: AssetItem[] = [];
  #snapshots: any[] = [];
  #exports: { files: any[]; records: any[] } = { files: [], records: [] };
  #selected: AssetItem | null = null;
  #busy = false;
  #message = "";
  #error = false;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #noteId: number | undefined;
  #noteText = "";
  #noteTime = "";
  #font: FontFace | null = null;

  constructor(readonly host: StudioHost, readonly shell: ValleStudioApp) {
    this.#services = new StudioServices(host.boot.session.token);
    this.#scope = host.boot.project ? "project" : "library";
    this.#panel.className = "studio-project-panel island";
    this.#panel.setAttribute("role", "dialog");
    this.#panel.setAttribute("aria-label", "Project panels");
    this.#panel.hidden = true;
    shell.append(this.#panel);
    document.getElementById("assetsPanelButton")!.addEventListener("click", () => this.open("assets"));
    const history = document.getElementById("historyPanelButton")!;
    history.hidden = !host.boot.project?.entry;
    history.addEventListener("click", () => this.open("history"));
    document.getElementById("homeButton")!.addEventListener("click", () => {
      void this.#run(async () => { await this.#ensureSaved(); location.assign("/?home=1"); });
    });
    if (host.boot.project?.entry) {
      document.getElementById("exportVideo")!.addEventListener("click", (event) => {
        event.stopImmediatePropagation(); event.preventDefault(); this.open("exports");
      }, { capture: true });
    }
    this.#panel.addEventListener("dragover", (event) => {
      if (this.host.boot.project && event.dataTransfer?.types.includes("Files")) event.preventDefault();
    });
    this.#panel.addEventListener("drop", (event) => {
      const files = [...event.dataTransfer?.files ?? []];
      if (files.length && this.host.boot.project) { event.preventDefault(); void this.#import(files); }
    });
    window.addEventListener("keydown",(event)=>{if(event.key==="Escape"&&!document.querySelector("dialog[open]"))this.#panel.hidden=true;});
    shell.addEventListener("studio-assets-changed", () => { if (!this.#panel.hidden) void this.#run(() => this.#refresh()); });
  }

  open(tab: Panel): void {
    this.#tab = tab; this.#panel.hidden = false; this.#message = "";
    void this.#run(() => this.#refresh());
  }

  async #run(action: () => Promise<void>): Promise<void> {
    if (this.#busy) return;
    this.#busy = true; this.#error = false; this.#render();
    try { await action(); }
    catch (error) { this.#message = error instanceof Error ? error.message : String(error); this.#error = true; }
    finally { this.#busy = false; this.#render(); }
  }

  async #ensureSaved(): Promise<void> {
    const saved = await new Promise<boolean>((complete) => {
      this.shell.dispatchEvent(new CustomEvent("studio-prepare-project-operation", { detail: { complete } }));
    });
    if (!saved) throw new Error("Save or resolve the current draft before this operation.");
  }

  async #refresh(): Promise<void> {
    if (this.#tab === "history") {
      this.#snapshots = (await this.#services.request("project", { op: "history" })).snapshots.toReversed();
    } else if (this.#tab === "exports") {
      this.#exports = await this.#services.request("project", { op: "exports" }) as { files: any[]; records: any[] };
    } else {
      const result = this.#scope === "project"
        ? await this.#services.request("project", { op: "show" })
        : await this.#services.request("library", { op: this.#query ? "search" : "list", ...(this.#query ? { query: this.#query, limit: 1000 } : {}),
          directory: this.#directory || undefined, kind: this.#kind || undefined, tag: this.#tag || undefined });
      this.#items = this.#scope === "project"
        ? (result.files as AssetItem[]).filter((item) => (!this.#kind || item.kind === this.#kind)
          && item.path.toLowerCase().includes(this.#query.toLowerCase()) && item.path.startsWith(this.#directory))
        : result.items;
      if (this.#selected) this.#selected = this.#items.find((item) => item.path === this.#selected!.path) ?? null;
    }
  }

  #search(): void {
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => { void this.#run(() => this.#refresh()); }, 250);
  }

  async #import(files: readonly File[]): Promise<void> {
    await this.#run(async () => {
      const items = await this.#services.importFiles(files); requireImported(items);
      this.#scope = "project"; this.#query = ""; this.#directory = ""; this.#kind = "";
      await this.#refresh(); this.#message = `Imported ${items.length} file${items.length === 1 ? "" : "s"}. Drag them onto the Timeline to create clips.`;
    });
  }

  async #addToTimeline(assets: readonly AssetItem[]): Promise<void> {
    await this.#run(async () => {
      const error = await new Promise<string | undefined>((complete) => {
        this.shell.dispatchEvent(new CustomEvent<ImportIntent>("studio-import-assets", { detail: { assets, complete } }));
      });
      if (error) throw new Error(error);
      this.#message = "Added and saved. Undo changes the Timeline; imported files remain in the project.";
      await this.#refresh();
    });
  }

  #select(item: AssetItem): void {
    this.#selected = item; this.#noteId = undefined; this.#noteText = ""; this.#noteTime = "";
    if (this.#font) document.fonts.delete(this.#font);
    this.#font = null;
    this.#render();
    if (item.kind === "font") {
      const font = new FontFace("valle-asset-sample", `url(${JSON.stringify(fileUrl(this.#scope, item.path))})`);
      void font.load().then(() => { if (this.#selected === item) { document.fonts.add(font); this.#font = font; this.#render(); } }).catch(() => {});
    }
  }

  #drag(event: DragEvent, item: AssetItem): void {
    event.dataTransfer?.setData("application/x-valle-asset", JSON.stringify(item));
    if (event.dataTransfer) event.dataTransfer.effectAllowed = "copy";
  }

  async #saveTags(event: SubmitEvent): Promise<void> {
    event.preventDefault(); const data = new FormData(event.currentTarget as HTMLFormElement);
    const tags = String(data.get("tags")).split(",").map((s) => s.trim()).filter(Boolean);
    const selected = this.#selected!;
    await this.#run(async () => { await this.#services.request("library", { op: "tag", path: selected.path,
      add: tags, rm: (selected.tags ?? []).filter((tag) => !tags.includes(tag)) }); await this.#refresh(); });
  }

  async #saveNote(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const text = this.#noteText, time = this.#noteTime.trim(), id = this.#noteId;
    await this.#run(async () => {
      const values=time ? time.split(",").map(Number) : [];
      if (values.length>2 || values.some(value=>!Number.isFinite(value))) throw new Error("Use seconds, or START,END for a range.");
      const timing = !time ? {} : time.includes(",") ? { range: values } : { at: values[0] };
      await this.#services.request("library", { op: "annotate", path: this.#selected!.path, text, id, ...timing });
      this.#noteText = ""; this.#noteTime = ""; this.#noteId = undefined; await this.#refresh();
    });
  }

  #seek(time: number): void { const media = this.#panel.querySelector<HTMLMediaElement>("audio,video"); if (media) media.currentTime = time; }

  #preview() {
    const item = this.#selected; if (!item) return null;
    const url = fileUrl(this.#scope, item.path);
    return html`<section class="asset-detail" aria-label="Asset details">
      <strong>${item.path}</strong><p class="muted">${item.kind}${item.info?.duration ? ` · ${item.info.duration.toFixed(2)} s` : ""}</p>
      ${item.kind === "image" ? html`<img class="asset-preview" src=${url} alt=${item.path}>` : null}
      ${item.kind === "audio" ? html`<audio class="asset-preview" controls src=${url}></audio>` : null}
      ${item.kind === "video" ? html`<video class="asset-preview" controls src=${url}></video>` : null}
      ${item.kind === "font" ? html`<p class="asset-font-sample" style="font-family:valle-asset-sample,serif">山川与光影<br>Valle Aa 012345</p><span>${item.info?.family ?? ""}</span>` : null}
      ${this.#scope === "library" && item.info?.duration ? html`<div class="asset-note-markers" aria-label="Timed notes">${(item.notes ?? []).filter((note) => note.at !== null || note.range).map((note) => html`<button title=${note.text} aria-label=${`Seek to ${note.text}`} style=${`left:${Math.min(98, 100 * (note.at ?? note.range![0]) / item.info!.duration!)}%`} @click=${() => this.#seek(note.at ?? note.range![0])}>◆</button>`)}</div>` : null}
      ${this.host.boot.project && this.host.boot.session.kind === "timeline-file" ? html`<button class="button" ?disabled=${this.#busy} @click=${() => this.#addToTimeline([item])}>Add to Timeline</button>` : null}
      ${this.#scope === "library" ? html`
        <form @submit=${(event: SubmitEvent) => this.#saveTags(event)}><label>Tags (comma separated)<input name="tags" aria-label="Asset tags" .value=${(item.tags ?? []).join(", ")}></label><button class="button quiet" ?disabled=${this.#busy}>Save tags</button></form>
        <ul class="asset-notes">${(item.notes ?? []).map((note) => html`<li><button class="note-text" @click=${() => this.#seek(note.at ?? note.range?.[0] ?? 0)}>${note.at !== null ? `${note.at}s · ` : note.range ? `${note.range.join("–")}s · ` : ""}${note.text}</button><button class="button quiet" title="Edit note" @click=${() => { this.#noteId = note.id; this.#noteText = note.text; this.#noteTime = note.at?.toString() ?? note.range?.join(",") ?? ""; this.#render(); }}>Edit</button><button class="button quiet" title="Delete note" @click=${() => this.#run(async () => { await this.#services.request("library", { op: "annotate", path: item.path, rm: note.id }); await this.#refresh(); })}>Delete</button></li>`)}</ul>
        <form @submit=${(event: SubmitEvent) => this.#saveNote(event)}><label>${this.#noteId ? "Edit note" : "New note"}<textarea aria-label="Note text" required .value=${this.#noteText} @input=${(event: Event) => { this.#noteText = (event.target as HTMLTextAreaElement).value; }}></textarea></label><label>Time in seconds (optional, or start,end)<input aria-label="Note time" .value=${this.#noteTime} @input=${(event: Event) => { this.#noteTime = (event.target as HTMLInputElement).value; }}></label><button class="button quiet" ?disabled=${this.#busy}>Save note</button></form>` : null}
    </section>`;
  }

  #assets() {
    return html`<div class="project-panel-tools">
      <div class="project-panel-tabs"><button class="button quiet" ?disabled=${!this.host.boot.project || this.#busy} aria-pressed=${this.#scope === "project"} @click=${() => { this.#scope = "project"; this.#selected = null; void this.#run(() => this.#refresh()); }}>Project files</button><button class="button quiet" ?disabled=${this.#busy} aria-pressed=${this.#scope === "library"} @click=${() => { this.#scope = "library"; this.#selected = null; void this.#run(() => this.#refresh()); }}>Library</button></div>
      <input aria-label="Search assets" placeholder="Search assets" .value=${this.#query} @input=${(event: Event) => { this.#query = (event.target as HTMLInputElement).value; this.#search(); }}>
      <div class="asset-filters"><input aria-label="Asset directory" placeholder="Directory" .value=${this.#directory} @input=${(event: Event) => { this.#directory = (event.target as HTMLInputElement).value; this.#search(); }}><select aria-label="Asset kind" .value=${this.#kind} @change=${(event: Event) => { this.#kind = (event.target as HTMLSelectElement).value; this.#search(); }}><option value="">All types</option>${kinds.map((kind) => html`<option value=${kind}>${kind}</option>`)}</select></div>
      ${this.#scope === "library" ? html`<input aria-label="Filter asset tag" placeholder="Tag" .value=${this.#tag} @input=${(event: Event) => { this.#tag = (event.target as HTMLInputElement).value; this.#search(); }}>` : null}
      ${this.host.boot.project ? html`<label class="button quiet import-files">Import files<input type="file" multiple aria-label="Import project files" ?disabled=${this.#busy} @change=${(event: Event) => { const input = event.target as HTMLInputElement; const files = [...input.files ?? []]; input.value = ""; void this.#import(files); }}></label><p class="muted">Drop files here to copy them into this project.</p>` : html`<p class="muted">Save as a project to import files and create clips.</p>`}
    </div><ul class="project-file-list" aria-label="Assets">${this.#items.map((item) => html`<li><button class="asset-row" draggable="true" data-asset-path=${item.path} aria-pressed=${this.#selected?.path === item.path} @dragstart=${(event: DragEvent) => this.#drag(event, item)} @click=${() => this.#select(item)}><span>${item.path}</span><small>${item.kind}${item.info?.duration ? ` · ${item.info.duration.toFixed(1)}s` : ""}</small></button></li>`)}</ul>${!this.#items.length && !this.#busy ? html`<p class="muted">No matching files.</p>` : null}${this.#preview()}`;
  }

  #history() {
    return html`<form class="project-panel-tools" @submit=${(event: SubmitEvent) => {
      event.preventDefault(); const form = event.currentTarget as HTMLFormElement; const message = new FormData(form).get("message");
      void this.#run(async () => { await this.#ensureSaved(); const report = await this.#services.request("project", { op: "save", message: message || undefined }); this.#message = report.unchanged ? `Snapshot ${report.snapshot} is unchanged.` : `Saved snapshot ${report.snapshot}.`; form.reset(); await this.#refresh(); });
    }}><label>Snapshot message<input name="message" aria-label="Snapshot message" placeholder="Optional"></label><button class="button" ?disabled=${this.#busy}>Save snapshot</button></form>
    <ul class="project-history-list">${this.#snapshots.map((snapshot) => html`<li><div><strong>Snapshot ${snapshot.number}</strong><p>${snapshot.message ?? snapshot.source}</p><small>${new Date(snapshot.createdAt).toLocaleString()}</small></div><button class="button quiet" ?disabled=${this.#busy} @click=${() => this.#run(async () => { await this.#ensureSaved(); const result = await this.#services.request("project", { op: "restore", snapshot: snapshot.number }); this.#message = `Restored ${snapshot.number}; previous sources are in snapshot ${result.beforeSnapshot}.${result.skipped?.length ? ` ${result.skipped.length} files were skipped; media and external files are not restored.` : ""}`; this.shell.dispatchEvent(new CustomEvent("studio-host-event", { detail: { type: "timeline", data: {} } })); await this.#refresh(); })}>Restore</button></li>`)}</ul>`;
  }

  #exportPanel() {
    return html`<section class="project-panel-tools"><p>Export the saved entry Timeline into renders/. Each full export records its source snapshot.</p><button class="button" ?disabled=${this.#busy} @click=${() => this.#run(async () => { await this.#ensureSaved(); this.#message = "Exporting…"; this.#render(); const result = await this.#services.request("project", { op: "render" }); this.#message = `Exported ${result.outputs.map((output: any) => output.path).join(", ")}${result.warnings?.length ? " · Media changed during export; see its record." : ""}`; await this.#refresh(); })}>Export video</button></section>
    <ul class="project-history-list">${this.#exports.files.map((file) => html`<li><a href=${fileUrl("project", file.path)} download>${file.path}</a><small>${(file.bytes / 1024 / 1024).toFixed(1)} MB</small></li>`)}</ul><h3>Export records</h3><ul class="project-history-list">${this.#exports.records.map((record) => html`<li><div><strong>Snapshot ${record.snapshot}</strong><p>${record.outputs.map((output: any) => output.path).join(", ")}</p><small>${new Date(record.createdAt).toLocaleString()}</small>${record.warnings?.map((warning: any) => html`<p>${warning.message}</p>`)}</div></li>`)}</ul>`;
  }

  #render(): void {
    render(html`<header class="project-panel-heading"><h2>${this.#tab === "assets" ? "Assets" : this.#tab === "history" ? "History" : "Exports"}</h2><button class="button quiet" aria-label="Close project panel" @click=${() => { this.#panel.hidden = true; }}>Close</button></header>
      <nav class="project-panel-tabs">${(["assets", ...(this.host.boot.project?.entry ? ["history", "exports"] : [])] as Panel[]).map((tab) => html`<button class="button quiet" aria-pressed=${this.#tab === tab} ?disabled=${this.#busy} @click=${() => this.open(tab)}>${tab === "assets" ? "Assets" : tab === "history" ? "History" : "Exports"}</button>`)}</nav>
      <p class=${`project-panel-message ${this.#error ? "error" : ""}`} role="status">${this.#message || (this.#busy ? "Loading…" : "")}</p>
      <div class="project-panel-content">${this.#tab === "assets" ? this.#assets() : this.#tab === "history" ? this.#history() : this.#exportPanel()}</div>`, this.#panel);
  }
}

export async function mountProjectHome(shell: HTMLElement, token: string): Promise<void> {
  shell.replaceChildren();
  const services = new StudioServices(token);
  let projects: any[] = [], directory = "", openDirectory = "", width="1920", height="1080", fps="30", error = "", busy = false;
  const act = async (command: object) => {
    if (busy) return; busy = true; error = ""; draw();
    try { const result = await services.request("project", command); location.assign(result.url); }
    catch (failure) { error = failure instanceof Error ? failure.message : String(failure); busy = false; draw(); }
  };
  const draw = () => render(html`<main class="studio-home"><header><span class="brand-lockup"></span><h1>Projects</h1></header>
    <section class="home-create island"><h2>Create a project</h2><form @submit=${(event: SubmitEvent) => { event.preventDefault(); const data = new FormData(event.currentTarget as HTMLFormElement); void act({ op: "create", directory: data.get("directory"), width: Number(data.get("width")), height: Number(data.get("height")), fps: data.get("fps") }); }}>
      <label>Project directory<input name="directory" aria-label="New project directory" required .value=${directory} @input=${(event:Event)=>{directory=(event.target as HTMLInputElement).value;}}></label><div class="home-canvas"><label>Width<input name="width" type="number" min="1" .value=${width} @input=${(event:Event)=>{width=(event.target as HTMLInputElement).value;}} required></label><label>Height<input name="height" type="number" min="1" .value=${height} @input=${(event:Event)=>{height=(event.target as HTMLInputElement).value;}} required></label><label>FPS<input name="fps" .value=${fps} @input=${(event:Event)=>{fps=(event.target as HTMLInputElement).value;}} required></label></div><button class="button" ?disabled=${busy}>Create project</button></form>
      <form class="home-open" @submit=${(event: SubmitEvent) => { event.preventDefault(); void act({ op: "open", directory: new FormData(event.currentTarget as HTMLFormElement).get("directory") }); }}><label>Open project directory<input name="directory" aria-label="Open project directory" .value=${openDirectory} @input=${(event:Event)=>{openDirectory=(event.target as HTMLInputElement).value;}} required></label><button class="button quiet" ?disabled=${busy}>Open</button></form>
    </section><p class="project-panel-message error" role="status">${error || (busy ? "Opening project…" : "")}</p><section><h2>Recent projects</h2><ul class="recent-projects">${projects.map((project) => html`<li><button class="island" ?disabled=${busy || project.missing} @click=${() => act({ op: "open", directory: project.path })}><strong>${project.name}</strong><span>${project.path}</span><small>${project.missing ? "Unavailable" : new Date(project.lastOpened).toLocaleString()}</small></button></li>`)}</ul>${!projects.length ? html`<p class="muted">Your recent projects will appear here.</p>` : null}</section></main>`, shell);
  try { const recent = await services.request("project", { op: "recent" }); projects = recent.projects; directory = `${recent.defaultDirectory.replace(/[\\/]$/, "")}/promo`; }
  catch (failure) { error = failure instanceof Error ? failure.message : String(failure); }
  draw();
}
