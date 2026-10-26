import { html, render } from "lit";
import { hydrateIcons } from "../../shared/icons.ts";
import type { StudioHost } from "./host.ts";
import type { ValleStudioApp } from "./studio-shell.ts";
import {
  StudioServices,
  requireImported,
  type AssetItem,
  type ImportIntent,
  type ProjectHistoryItem,
  type ProjectExports,
  type ProjectExportResult,
  type RecentProject,
} from "./project-client.ts";

type Panel = "assets" | "history" | "exports" | "export-history";
const kinds = [
  "video",
  "audio",
  "image",
  "font",
  "model3d",
  "environment",
  "shader",
  "component",
  "data",
  "other",
];
const kindLabels: Record<string, string> = {
  video: "Video",
  audio: "Audio",
  image: "Image",
  font: "Font",
  model3d: "3D model",
  environment: "Environment",
  shader: "Shader",
  component: "Component",
  data: "Data",
  other: "File",
};
const fileUrl = (scope: "project" | "library", path: string) =>
  `${scope === "project" ? "/project/file/" : "/library/blob/"}${path.split("/").map(encodeURIComponent).join("/")}`;

export class ProjectPanels {
  readonly #services: StudioServices;
  readonly #panel = document.createElement("aside");
  readonly #dialog = document.createElement("dialog");
  #tab: Panel = "assets";
  #scope: "project" | "library";
  #query = "";
  #kind = "";
  #directory = "";
  #tag = "";
  #items: AssetItem[] = [];
  #snapshots: ProjectHistoryItem[] = [];
  #exports: ProjectExports = { files: [], records: [] };
  #selected: AssetItem | null = null;
  #busy = false;
  #message = "";
  #error = false;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #noteId: number | undefined;
  #noteText = "";
  #noteTime = "";
  #font: FontFace | null = null;
  #filtersOpen = false;
  #lastExport: ProjectExportResult | null = null;

  constructor(
    readonly host: StudioHost,
    readonly shell: ValleStudioApp,
  ) {
    this.#services = new StudioServices(host.boot.session.token);
    this.#scope = host.boot.project ? "project" : "library";
    this.#panel.className = "studio-project-panel island";
    this.#panel.setAttribute("role", "region");
    this.#panel.hidden = true;
    shell.querySelector(".workspace")!.prepend(this.#panel);
    this.#dialog.className = "studio-dialog project-tools-dialog";
    this.#dialog.id = "projectToolsDialog";
    this.#dialog.setAttribute("aria-labelledby", "projectPanelTitle");
    this.#dialog.addEventListener("close", () => {
      if (!this.#dialog.open && this.#panel.parentElement === this.#dialog) this.close();
    });
    shell.append(this.#dialog);
    document.getElementById("assetsPanelButton")!.addEventListener("click", () => {
      if (this.#tab === "assets" && !this.#panel.hidden) this.close();
      else this.open("assets");
    });
    const history = document.getElementById("historyPanelButton")!;
    history.hidden = !host.boot.project?.entry;
    history.addEventListener("click", () => this.open("history"));
    const exports = document.getElementById("exportHistoryButton")!;
    exports.hidden = !host.boot.project?.entry;
    exports.addEventListener("click", () => this.open("export-history"));
    document.getElementById("homeButton")!.addEventListener("click", (event) => {
      event.preventDefault();
      void this.#run(async () => {
        await this.#ensureSaved();
        location.assign("/?home=1");
      });
    });
    if (host.boot.project?.entry) {
      document.getElementById("exportVideo")!.setAttribute("aria-controls", this.#dialog.id);
      document.getElementById("exportVideo")!.addEventListener(
        "click",
        (event) => {
          event.stopImmediatePropagation();
          event.preventDefault();
          this.open("exports");
        },
        { capture: true },
      );
    }
    this.#panel.addEventListener("dragover", (event) => {
      if (this.host.boot.project && event.dataTransfer?.types.includes("Files"))
        event.preventDefault();
    });
    this.#panel.addEventListener("drop", (event) => {
      const files = [...(event.dataTransfer?.files ?? [])];
      if (files.length && this.host.boot.project) {
        event.preventDefault();
        void this.#import(files);
      }
    });
    window.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && !document.querySelector("dialog[open]")) this.close();
    });
    shell.addEventListener("studio-assets-changed", () => {
      if (!this.#panel.hidden) void this.#run(() => this.#refresh());
    });
  }

  open(tab: Panel): void {
    this.#tab = tab;
    this.#panel.dataset.panel = tab;
    this.#panel.setAttribute(
      "aria-label",
      tab === "assets" ? "Assets" : tab === "history" ? "History" : "Export",
    );
    if (tab === "assets") {
      if (innerWidth <= 1024) this.shell.classList.remove("mobile-inspector");
      if (this.#dialog.open) this.#dialog.close();
      this.shell.querySelector(".workspace")!.prepend(this.#panel);
    } else {
      this.#dialog.append(this.#panel);
    }
    this.#panel.hidden = false;
    document
      .getElementById("assetsPanelButton")!
      .setAttribute("aria-pressed", String(tab === "assets"));
    this.#message = "";
    this.#render();
    if (tab !== "assets" && !this.#dialog.open) this.#dialog.showModal();
    void this.#run(() => this.#refresh());
  }

  close(): void {
    this.#panel.hidden = true;
    document.getElementById("assetsPanelButton")!.setAttribute("aria-pressed", "false");
    if (this.#dialog.open) this.#dialog.close();
  }

  async #run(action: () => Promise<void>): Promise<void> {
    if (this.#busy) return;
    this.#busy = true;
    this.#error = false;
    this.#render();
    try {
      await action();
    } catch (error) {
      this.#message = error instanceof Error ? error.message : String(error);
      this.#error = true;
    } finally {
      this.#busy = false;
      this.#render();
    }
  }

  async #ensureSaved(): Promise<void> {
    const saved = await new Promise<boolean>((complete) => {
      this.shell.dispatchEvent(
        new CustomEvent("studio-prepare-project-operation", { detail: { complete } }),
      );
    });
    if (!saved) throw new Error("Save or resolve the current draft before this operation.");
  }

  async #refresh(): Promise<void> {
    if (this.#tab === "history") {
      this.#snapshots = (
        await this.#services.request<{ snapshots: ProjectHistoryItem[] }>("project", {
          op: "history",
        })
      ).snapshots.toReversed();
    } else if (this.#tab === "exports" || this.#tab === "export-history") {
      this.#exports = await this.#services.request<ProjectExports>("project", { op: "exports" });
    } else {
      if (this.#scope === "project") {
        const result = await this.#services.request<{ files: AssetItem[] }>("project", {
          op: "show",
        });
        this.#items = result.files.filter(
          (item) =>
            (!this.#kind || item.kind === this.#kind) &&
            item.path.toLowerCase().includes(this.#query.toLowerCase()) &&
            item.path.startsWith(this.#directory),
        );
      } else {
        const result = await this.#services.request<{ items: AssetItem[] }>("library", {
          op: this.#query ? "search" : "list",
          ...(this.#query ? { query: this.#query, limit: 1000 } : {}),
          directory: this.#directory || undefined,
          kind: this.#kind || undefined,
          tag: this.#tag || undefined,
        });
        this.#items = result.items;
      }
      if (this.#selected)
        this.#selected = this.#items.find((item) => item.path === this.#selected!.path) ?? null;
    }
  }

  #search(): void {
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => {
      void this.#run(() => this.#refresh());
    }, 250);
  }

  async #import(files: readonly File[]): Promise<void> {
    await this.#run(async () => {
      const items = await this.#services.importFiles(files);
      requireImported(items);
      this.#scope = "project";
      this.#query = "";
      this.#directory = "";
      this.#kind = "";
      await this.#refresh();
      this.#message = `Imported ${items.length} file${items.length === 1 ? "" : "s"}.`;
    });
  }

  async #addToTimeline(assets: readonly AssetItem[]): Promise<void> {
    await this.#run(async () => {
      const error = await new Promise<string | undefined>((complete) => {
        this.shell.dispatchEvent(
          new CustomEvent<ImportIntent>("studio-import-assets", { detail: { assets, complete } }),
        );
      });
      if (error) throw new Error(error);
      this.#message = "Added to timeline.";
      await this.#refresh();
    });
  }

  #select(item: AssetItem): void {
    this.#selected = item;
    this.#noteId = undefined;
    this.#noteText = "";
    this.#noteTime = "";
    if (this.#font) document.fonts.delete(this.#font);
    this.#font = null;
    this.#render();
    if (item.kind === "font") {
      const font = new FontFace(
        "valle-asset-sample",
        `url(${JSON.stringify(fileUrl(this.#scope, item.path))})`,
      );
      void font
        .load()
        .then(() => {
          if (this.#selected === item) {
            document.fonts.add(font);
            this.#font = font;
            this.#render();
          }
        })
        .catch(() => {});
    }
  }

  #drag(event: DragEvent, item: AssetItem): void {
    event.dataTransfer?.setData("application/x-valle-asset", JSON.stringify(item));
    if (event.dataTransfer) event.dataTransfer.effectAllowed = "copy";
  }

  async #saveTags(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const data = new FormData(event.currentTarget as HTMLFormElement);
    const tags = String(data.get("tags"))
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    const selected = this.#selected!;
    await this.#run(async () => {
      await this.#services.request("library", {
        op: "tag",
        path: selected.path,
        add: tags,
        rm: (selected.tags ?? []).filter((tag) => !tags.includes(tag)),
      });
      await this.#refresh();
    });
  }

  async #saveNote(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const text = this.#noteText,
      time = this.#noteTime.trim(),
      id = this.#noteId;
    await this.#run(async () => {
      const values = time ? time.split(",").map(Number) : [];
      if (values.length > 2 || values.some((value) => !Number.isFinite(value)))
        throw new Error("Use seconds, or START,END for a range.");
      const timing = !time ? {} : time.includes(",") ? { range: values } : { at: values[0] };
      await this.#services.request("library", {
        op: "annotate",
        path: this.#selected!.path,
        text,
        id,
        ...timing,
      });
      this.#noteText = "";
      this.#noteTime = "";
      this.#noteId = undefined;
      await this.#refresh();
    });
  }

  #seek(time: number): void {
    const media = this.#panel.querySelector<HTMLMediaElement>("audio,video");
    if (media) media.currentTime = time;
  }

  #preview() {
    const item = this.#selected;
    if (!item) return null;
    const url = fileUrl(this.#scope, item.path);
    return html`<section class="asset-detail" aria-label="Asset details">
      <strong title=${item.path}>${item.path.split(/[\\/]/).pop()}</strong>
      ${item.kind === "image" ? html`<img class="asset-preview" src=${url} alt=${item.path} />` : null}
      ${item.kind === "audio" ? html`<audio class="asset-preview" controls src=${url}></audio>` : null}
      ${item.kind === "video" ? html`<video class="asset-preview" controls src=${url}></video>` : null}
      ${
        item.kind === "font"
          ? html`<p class="asset-font-sample" style="font-family:valle-asset-sample,serif">
                山川与光影<br />Valle Aa 012345
              </p>
              <span>${item.info?.family ?? ""}</span>`
          : null
      }
      ${this.#scope === "library" && item.info?.duration ? html`<div class="asset-note-markers" aria-label="Timed notes">${(item.notes ?? []).filter((note) => note.at !== null || note.range).map((note) => html`<button title=${note.text} aria-label=${`Seek to ${note.text}`} style=${`left:${Math.min(98, (100 * (note.at ?? note.range![0])) / item.info!.duration!)}%`} @click=${() => this.#seek(note.at ?? note.range![0])}>◆</button>`)}</div>` : null}
      ${this.host.boot.project && this.host.boot.session.kind === "timeline-file" ? html`<button class="button" ?disabled=${this.#busy} @click=${() => this.#addToTimeline([item])}>Add to Timeline</button>` : null}
      ${
        this.#scope === "library"
          ? html` <form @submit=${(event: SubmitEvent) => this.#saveTags(event)}>
                <label
                  >Tags (comma separated)<input
                    name="tags"
                    aria-label="Asset tags"
                    .value=${(item.tags ?? []).join(", ")} /></label
                ><button class="button quiet" ?disabled=${this.#busy}>Save tags</button>
              </form>
              <ul class="asset-notes">
                ${(item.notes ?? []).map(
                  (note) =>
                    html`<li>
                      <button
                        class="note-text"
                        @click=${() => this.#seek(note.at ?? note.range?.[0] ?? 0)}
                      >
                        ${note.at !== null ? `${note.at}s · ` : note.range ? `${note.range.join("–")}s · ` : ""}${note.text}</button
                      ><button
                        class="button quiet"
                        title="Edit note"
                        @click=${() => {
                          this.#noteId = note.id;
                          this.#noteText = note.text;
                          this.#noteTime = note.at?.toString() ?? note.range?.join(",") ?? "";
                          this.#render();
                        }}
                      >
                        Edit</button
                      ><button
                        class="button quiet"
                        title="Delete note"
                        @click=${() =>
                          this.#run(async () => {
                            await this.#services.request("library", {
                              op: "annotate",
                              path: item.path,
                              rm: note.id,
                            });
                            await this.#refresh();
                          })}
                      >
                        Delete
                      </button>
                    </li>`,
                )}
              </ul>
              <form @submit=${(event: SubmitEvent) => this.#saveNote(event)}>
                <label
                  >${this.#noteId ? "Edit note" : "New note"}<textarea
                    aria-label="Note text"
                    required
                    .value=${this.#noteText}
                    @input=${(event: Event) => {
                      this.#noteText = (event.target as HTMLTextAreaElement).value;
                    }}
                  ></textarea></label
                ><label
                  >Time in seconds (optional, or start,end)<input
                    aria-label="Note time"
                    .value=${this.#noteTime}
                    @input=${(event: Event) => {
                      this.#noteTime = (event.target as HTMLInputElement).value;
                    }} /></label
                ><button class="button quiet" ?disabled=${this.#busy}>Save note</button>
              </form>`
          : null
      }
    </section>`;
  }

  #assetScopes() {
    return html`<div class="project-panel-tabs" role="group" aria-label="Asset source">
      <button
        class="button quiet"
        ?disabled=${!this.host.boot.project || this.#busy}
        aria-pressed=${this.#scope === "project"}
        @click=${() => {
          this.#scope = "project";
          this.#selected = null;
          void this.#run(() => this.#refresh());
        }}
      >
        Project</button
      ><button
        class="button quiet"
        ?disabled=${this.#busy}
        aria-pressed=${this.#scope === "library"}
        @click=${() => {
          this.#scope = "library";
          this.#selected = null;
          void this.#run(() => this.#refresh());
        }}
      >
        Library
      </button>
    </div>`;
  }

  #assets() {
    return html`<div class="project-panel-tools">
        <input
          aria-label="Search assets"
          placeholder="Search assets"
          .value=${this.#query}
          @input=${(event: Event) => {
            this.#query = (event.target as HTMLInputElement).value;
            this.#search();
          }}
        />
        <div class="asset-filter-bar">
          <select
            aria-label="Asset kind"
            .value=${this.#kind}
            @change=${(event: Event) => {
              this.#kind = (event.target as HTMLSelectElement).value;
              this.#search();
            }}
          >
            <option value="">All types</option>
            ${kinds.map((kind) => html`<option value=${kind}>${kindLabels[kind]}</option>`)}
          </select>
          ${
            this.host.boot.project
              ? html`<label class="button quiet import-files">
                  Import
                  <input
                    class="sr-only"
                    type="file"
                    multiple
                    aria-label="Import project files"
                    ?disabled=${this.#busy}
                    @change=${(event: Event) => {
                      const input = event.target as HTMLInputElement;
                      const files = [...(input.files ?? [])];
                      input.value = "";
                      void this.#import(files);
                    }}
                  />
                </label>`
              : null
          }
        </div>
        <details
          class="asset-filters"
          ?open=${this.#filtersOpen}
          @toggle=${(event: Event) => {
            this.#filtersOpen = (event.currentTarget as HTMLDetailsElement).open;
          }}
        >
          <summary>Filters${this.#directory || this.#tag ? " · Active" : ""}</summary>
          <input
            aria-label="Asset directory"
            placeholder="Directory"
            .value=${this.#directory}
            @input=${(event: Event) => {
              this.#directory = (event.target as HTMLInputElement).value;
              this.#search();
            }}
          />
          ${
            this.#scope === "library"
              ? html`<input
                  aria-label="Filter asset tag"
                  placeholder="Tag"
                  .value=${this.#tag}
                  @input=${(event: Event) => {
                    this.#tag = (event.target as HTMLInputElement).value;
                    this.#search();
                  }}
                />`
              : null
          }
        </details>
      </div>
      <ul class="project-file-list" aria-label="Assets">
        ${this.#items.map(
          (item) =>
            html`<li>
              <button
                class="asset-row"
                draggable="true"
                title=${item.path}
                data-asset-path=${item.path}
                aria-pressed=${this.#selected?.path === item.path}
                @dragstart=${(event: DragEvent) => this.#drag(event, item)}
                @click=${() => this.#select(item)}
              >
                <span>${item.path.split(/[\\/]/).pop()}</span
                ><small
                  >${kindLabels[item.kind] ?? "File"}${item.info?.duration ? ` · ${item.info.duration.toFixed(1)}s` : ""}</small
                >
              </button>
            </li>`,
        )}
      </ul>
      ${!this.#items.length && !this.#busy ? html`<p class="asset-empty muted">${this.#query || this.#kind || this.#directory || this.#tag ? "No matching files" : this.host.boot.project ? "Drop files here to get started" : "No assets yet"}</p>` : null}${this.#preview()}`;
  }

  #history() {
    return html`<form
        class="project-panel-tools"
        @submit=${(event: SubmitEvent) => {
          event.preventDefault();
          const form = event.currentTarget as HTMLFormElement;
          const message = new FormData(form).get("message");
          void this.#run(async () => {
            await this.#ensureSaved();
            const report = await this.#services.request<{ snapshot: number; unchanged: boolean }>(
              "project",
              { op: "save", message: message || undefined },
            );
            this.#message = report.unchanged
              ? `Snapshot ${report.snapshot} is unchanged.`
              : `Saved snapshot ${report.snapshot}.`;
            form.reset();
            await this.#refresh();
          });
        }}
      >
        <label
          >Snapshot message<input
            name="message"
            aria-label="Snapshot message"
            placeholder="Optional" /></label
        ><button class="button" ?disabled=${this.#busy}>Save snapshot</button>
      </form>
      <ul class="project-history-list">
        ${this.#snapshots.map(
          (snapshot) =>
            html`<li>
              <div>
                <strong>Snapshot ${snapshot.number}</strong>
                <p>${snapshot.message ?? snapshot.source}</p>
                <small>${new Date(snapshot.createdAt).toLocaleString()}</small>
              </div>
              <button
                class="button quiet"
                ?disabled=${this.#busy}
                @click=${() =>
                  this.#run(async () => {
                    await this.#ensureSaved();
                    const result = await this.#services.request<{
                      beforeSnapshot: number;
                      skipped?: unknown[];
                    }>("project", { op: "restore", snapshot: snapshot.number });
                    this.#message = `Restored ${snapshot.number}; previous sources are in snapshot ${result.beforeSnapshot}.${result.skipped?.length ? ` ${result.skipped.length} files were skipped; media and external files are not restored.` : ""}`;
                    this.shell.dispatchEvent(
                      new CustomEvent("studio-host-event", {
                        detail: { type: "timeline", data: {} },
                      }),
                    );
                    await this.#refresh();
                  })}
              >
                Restore
              </button>
            </li>`,
        )}
      </ul>`;
  }

  #exportPanel() {
    return html`<section class="native-export-body">
        <dl class="project-settings-facts">
          ${[
            ["Resolution", "projectCanvas"],
            ["Frame rate", "projectFps"],
            ["Duration", "projectDuration"],
          ].map(
            ([label, id]) =>
              html`<div>
                <dt>${label}</dt>
                <dd>${document.getElementById(id!)?.textContent ?? "—"}</dd>
              </div>`,
          )}
        </dl>
        ${this.#busy ? html`<progress aria-label="Export progress"></progress>` : null}
        ${this.#lastExport?.outputs.map((output) => html`<a class="export-result" href=${fileUrl("project", output.path)} download><span data-icon="download"></span>${output.path.split("/").pop()}</a>`)}
      </section>
      <footer class="native-export-actions">
        <button
          class="button primary"
          ?disabled=${this.#busy}
          @click=${() =>
            this.#run(async () => {
              await this.#ensureSaved();
              this.#lastExport = null;
              this.#message = "Exporting…";
              this.#render();
              const result = await this.#services.request<ProjectExportResult>("project", {
                op: "render",
              });
              this.#lastExport = result;
              this.#message = result.warnings?.length
                ? "Video exported. Media changed during export; check export history."
                : "Video exported.";
              await this.#refresh();
            })}
        >
          Export video
        </button>
      </footer>`;
  }

  #exportHistory() {
    return html` <ul class="project-history-list">
        ${this.#exports.files.map((file) => {
          const record = this.#exports.records.find((record) =>
            record.outputs.some((output) => output.path === file.path),
          );
          return html`<li>
            <div>
              <a href=${fileUrl("project", file.path)} title=${file.path} download
                >${file.path.split("/").pop()}</a
              >${record ? html`<small>${new Date(record.createdAt).toLocaleString()}</small>${record.warnings?.map((warning) => html`<p>${warning.message}</p>`)}` : null}
            </div>
            <small>${(file.bytes / 1024 / 1024).toFixed(1)} MB</small>
          </li>`;
        })}
      </ul>
      ${!this.#exports.files.length && !this.#busy ? html`<p class="muted">No exports yet</p>` : null}`;
  }

  #render(): void {
    const title =
      this.#tab === "assets"
        ? "Assets"
        : this.#tab === "history"
          ? "History"
          : this.#tab === "exports"
            ? "Export video"
            : "Export history";
    this.#panel.setAttribute("aria-busy", String(this.#busy));
    render(
      html`<header class="project-panel-heading">
          <h2 id="projectPanelTitle" class=${this.#tab === "assets" ? "sr-only" : ""}>${title}</h2>
          ${this.#tab === "assets" ? this.#assetScopes() : null}
          <button
            class="icon-button"
            aria-label="Close project panel"
            title="Close"
            @click=${() => this.close()}
          >
            <span data-icon="close"></span>
          </button>
        </header>
        <p class=${`project-panel-message ${this.#error ? "error" : ""}`} role="status">
          ${this.#message || (this.#busy ? "Loading…" : "")}
        </p>
        <div class="project-panel-content">
          ${this.#tab === "assets" ? this.#assets() : this.#tab === "history" ? this.#history() : this.#tab === "exports" ? this.#exportPanel() : this.#exportHistory()}
        </div>`,
      this.#panel,
    );
    hydrateIcons(this.#panel);
  }
}

export async function mountProjectHome(shell: HTMLElement, token: string): Promise<void> {
  shell.replaceChildren();
  const services = new StudioServices(token);
  let projects: RecentProject[] = [],
    directory = "",
    openDirectory = "",
    width = "1920",
    height = "1080",
    fps = "30",
    resolution = "1080",
    error = "",
    busy = false;
  const act = async (command: object) => {
    if (busy) return;
    busy = true;
    error = "";
    draw();
    try {
      const result = await services.request<{ url: string }>("project", command);
      location.assign(result.url);
    } catch (failure) {
      error = failure instanceof Error ? failure.message : String(failure);
      busy = false;
      draw();
    }
  };
  const draw = () => {
    render(
      html`<main class="studio-home">
        <header>
          <div class="home-heading">
            <span class="brand-lockup"></span>
            <h1>Projects</h1>
          </div>
          <div class="home-actions">
            <button
              class="button quiet"
              ?disabled=${busy}
              @click=${() => shell.querySelector<HTMLDialogElement>("#homeOpenDialog")!.showModal()}
            >
              Open project
            </button>
            <button
              class="button primary"
              ?disabled=${busy}
              @click=${() => shell.querySelector<HTMLDialogElement>("#homeCreateDialog")!.showModal()}
            >
              New project
            </button>
          </div>
        </header>
        <p class="project-panel-message error" role="status">${error}</p>
        <section class="home-recents">
          <h2>Recent</h2>
          <ul class="recent-projects">
            ${projects.map(
              (project) =>
                html`<li>
                  <button
                    class="island"
                    title=${project.path}
                    ?disabled=${busy || project.missing}
                    @click=${() => act({ op: "open", directory: project.path })}
                  >
                    <strong>${project.name}</strong><span>${project.path}</span>
                    <small
                      >${project.missing ? "Unavailable" : new Date(project.lastOpened).toLocaleDateString()}</small
                    >
                  </button>
                </li>`,
            )}
          </ul>
          ${
            !projects.length
              ? html`<div class="home-empty">
                  <p>No recent projects</p>
                  <button
                    class="button quiet"
                    ?disabled=${busy}
                    @click=${() => shell.querySelector<HTMLDialogElement>("#homeCreateDialog")!.showModal()}
                  >
                    Create your first project
                  </button>
                </div>`
              : null
          }
        </section>
        <dialog
          id="homeCreateDialog"
          class="studio-dialog home-project-dialog"
          aria-labelledby="homeCreateTitle"
        >
          <header class="project-panel-heading">
            <h2 id="homeCreateTitle">Create a project</h2>
            <button
              class="icon-button"
              type="button"
              aria-label="Close new project"
              @click=${() => shell.querySelector<HTMLDialogElement>("#homeCreateDialog")!.close()}
            >
              <span data-icon="close"></span>
            </button>
          </header>
          <form
            @submit=${(event: SubmitEvent) => {
              event.preventDefault();
              const data = new FormData(event.currentTarget as HTMLFormElement);
              void act({
                op: "create",
                directory: data.get("directory"),
                width: Number(width),
                height: Number(height),
                fps: data.get("fps"),
              });
            }}
          >
            <label
              >Location<input
                name="directory"
                aria-label="New project directory"
                required
                .value=${directory}
                @input=${(event: Event) => {
                  directory = (event.target as HTMLInputElement).value;
                }}
            /></label>
            <div class="home-presets">
              <label
                >Resolution<select
                  aria-label="Project resolution"
                  .value=${resolution}
                  @change=${(event: Event) => {
                    resolution = (event.target as HTMLSelectElement).value;
                    const size = {
                      "2160": [3840, 2160],
                      "1080": [1920, 1080],
                      "720": [1280, 720],
                      portrait: [1080, 1920],
                      square: [1080, 1080],
                    }[resolution];
                    if (size) {
                      width = String(size[0]);
                      height = String(size[1]);
                    }
                    draw();
                  }}
                >
                  <option value="2160">4K · 3840 × 2160</option>
                  <option value="1080">1080p · 1920 × 1080</option>
                  <option value="720">720p · 1280 × 720</option>
                  <option value="portrait">Portrait · 1080 × 1920</option>
                  <option value="square">Square · 1080 × 1080</option>
                  <option value="custom">Custom</option>
                </select></label
              >
              <label
                >Frame rate<input
                  name="fps"
                  .value=${fps}
                  required
                  @input=${(event: Event) => {
                    fps = (event.target as HTMLInputElement).value;
                  }}
              /></label>
            </div>
            ${
              resolution === "custom"
                ? html`<div class="home-canvas">
                    <label
                      >Width<input
                        name="width"
                        type="number"
                        min="1"
                        .value=${width}
                        @input=${(event: Event) => {
                          width = (event.target as HTMLInputElement).value;
                        }}
                        required /></label
                    ><label
                      >Height<input
                        name="height"
                        type="number"
                        min="1"
                        .value=${height}
                        @input=${(event: Event) => {
                          height = (event.target as HTMLInputElement).value;
                        }}
                        required
                    /></label>
                  </div>`
                : null
            }
            ${error ? html`<p class="error" role="alert">${error}</p>` : null}
            <div class="dialog-actions">
              <button class="button primary" ?disabled=${busy}>
                ${busy ? "Creating…" : "Create project"}
              </button>
            </div>
          </form>
        </dialog>
        <dialog
          id="homeOpenDialog"
          class="studio-dialog home-project-dialog"
          aria-labelledby="homeOpenTitle"
        >
          <header class="project-panel-heading">
            <h2 id="homeOpenTitle">Open project</h2>
            <button
              class="icon-button"
              type="button"
              aria-label="Close open project"
              @click=${() => shell.querySelector<HTMLDialogElement>("#homeOpenDialog")!.close()}
            >
              <span data-icon="close"></span>
            </button>
          </header>
          <form
            @submit=${(event: SubmitEvent) => {
              event.preventDefault();
              void act({
                op: "open",
                directory: new FormData(event.currentTarget as HTMLFormElement).get("directory"),
              });
            }}
          >
            <label
              >Location<input
                name="directory"
                aria-label="Open project directory"
                .value=${openDirectory}
                @input=${(event: Event) => {
                  openDirectory = (event.target as HTMLInputElement).value;
                }}
                required /></label
            >${error ? html`<p class="error" role="alert">${error}</p>` : null}
            <div class="dialog-actions">
              <button class="button primary" ?disabled=${busy}>
                ${busy ? "Opening…" : "Open"}
              </button>
            </div>
          </form>
        </dialog>
      </main>`,
      shell,
    );
    hydrateIcons(shell);
  };
  try {
    const recent = await services.request<{ projects: RecentProject[]; defaultDirectory: string }>(
      "project",
      { op: "recent" },
    );
    projects = recent.projects;
    directory = `${recent.defaultDirectory.replace(/[\\/]$/, "")}/promo`;
  } catch (failure) {
    error = failure instanceof Error ? failure.message : String(failure);
  }
  draw();
}
