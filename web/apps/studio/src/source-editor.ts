import type { StudioHost } from "./host.ts";
import { SourceDrafts, type SourceSaveReport } from "./source-drafts.ts";

function required<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing Studio source element #${id}`);
  return element as T;
}

/** One source draft set shared by the current Studio session and save flow. */
export class StudioSourceEditor extends EventTarget {
  drafts: SourceDrafts;
  readonly #host: StudioHost;
  readonly #toggle = required<HTMLButtonElement>("sourceToggle");
  readonly #pane = required<HTMLElement>("sourcePane");
  readonly #previewPane = required<HTMLElement>("sourcePane").parentElement!;
  readonly #selector = required<HTMLSelectElement>("sourceFile");
  readonly #textarea = required<HTMLTextAreaElement>("sourceText");
  readonly #status = required<HTMLElement>("sourceStatus");
  readonly #saveButton = required<HTMLButtonElement>("sourceSave");
  readonly #reloadButton = required<HTMLButtonElement>("sourceReload");
  #selected: string | null = null;
  #saving = false;
  #refreshAfterSave = false;
  #refreshVersion = 0;
  #draftDiscoveryTimer: ReturnType<typeof setTimeout> | null = null;

  private constructor(host: StudioHost, drafts: SourceDrafts) {
    super();
    this.#host = host;
    this.drafts = drafts;
    this.#toggle.addEventListener("click", () => this.toggle());
    this.#selector.addEventListener("change", () => this.open(this.#selector.value));
    this.#textarea.addEventListener("input", () => {
      if (!this.#selected) return;
      this.dispatchEvent(new Event("before-edit"));
      this.drafts.edit(this.#selected, this.#textarea.value);
      this.#renderStatus();
      this.dispatchEvent(new Event("edited"));
      this.#discoverImportsSoon();
    });
    this.#saveButton.addEventListener("click", () => this.dispatchEvent(new Event("save-request")));
    this.#reloadButton.addEventListener("click", () => void this.reload());
    this.#renderFiles();
  }

  static async mount(host: StudioHost): Promise<StudioSourceEditor> {
    const files = await host.loadSourceFiles();
    return new StudioSourceEditor(host, new SourceDrafts(files));
  }

  get dirty(): boolean { return this.drafts.dirty; }
  get conflicted(): boolean { return [...this.drafts.files.values()].some((file) => file.conflict !== null); }

  confirmedDigests(): Record<string, string> { return this.drafts.confirmedDigests(); }

  snapshot(): Record<string, string> { return this.drafts.snapshot(); }

  resolvePath(path: string): string {
    if (this.drafts.files.has(path)) return path;
    const matches = [...this.drafts.files.keys()].filter((candidate) => candidate.endsWith(`/${path}`));
    if (matches.length === 1) return matches[0]!;
    const basename = path.split(/[\\/]/).pop();
    const byName = [...this.drafts.files.keys()].filter((candidate) => candidate.endsWith(`/${basename}`));
    if (byName.length !== 1) throw new Error(`Source path is unavailable or ambiguous: ${path}`);
    return byName[0]!;
  }

  restore(snapshot: Record<string, string>): void {
    this.drafts.restore(snapshot);
    this.#renderSelected();
    this.dispatchEvent(new Event("restored"));
  }

  toggle(): void {
    const open = this.#pane.hidden !== false;
    this.#pane.hidden = !open;
    this.#previewPane.classList.toggle("source-open", open);
    this.#toggle.setAttribute("aria-pressed", String(open));
    if (open) this.#textarea.focus();
  }

  open(path: string): void {
    if (!this.drafts.files.has(path)) return;
    this.#selected = path;
    this.#selector.value = path;
    if (this.#pane.hidden) this.toggle();
    this.#renderSelected();
  }

  replace(path: string, text: string): void {
    const current = this.drafts.files.get(path);
    if (!current) throw new Error(`Source is not loaded: ${path}`);
    if (current.text === text) return;
    this.dispatchEvent(new Event("before-edit"));
    this.drafts.edit(path, text);
    this.#renderSelected();
    this.dispatchEvent(new Event("edited"));
    this.#discoverImportsSoon();
  }

  async saveDirty(): Promise<SourceSaveReport> {
    if (this.#saving) return { saved: {}, failed: { path: this.#selected ?? "", message: "Source save already in progress" } };
    this.#saving = true;
    this.#renderStatus();
    try {
      const report = await this.drafts.saveDirty(this.#host);
      this.#renderStatus();
      this.dispatchEvent(new CustomEvent<SourceSaveReport>("saved", { detail: report }));
      return report;
    } finally {
      this.#saving = false;
      this.#renderStatus();
      if (this.#refreshAfterSave) {
        this.#refreshAfterSave = false;
        void this.refreshFromDisk();
      }
    }
  }

  async refreshFromDisk(): Promise<void> {
    if (this.#saving) {
      this.#refreshAfterSave = true;
      return;
    }
    const version = ++this.#refreshVersion;
    try {
      const draftImports = Object.fromEntries([...this.drafts.files.values()]
        .filter((file) => file.text !== file.savedText)
        .map((file) => [file.path, file.text]));
      const files = await this.#host.loadSourceFiles(draftImports);
      if (version !== this.#refreshVersion) return;
      const changed = this.drafts.reconcile(files);
      this.#renderFiles();
      if (changed) this.dispatchEvent(new Event("reconciled"));
    } catch (error) {
      if (version !== this.#refreshVersion) return;
      this.#status.textContent = error instanceof Error ? error.message : String(error);
    }
  }

  async reload(): Promise<boolean> {
    if (this.dirty && !confirm("Discard unsaved source edits and reload files from disk?")) return false;
    try {
      ++this.#refreshVersion;
      const files = await this.#host.loadSourceFiles();
      this.drafts = new SourceDrafts(files);
      this.#renderFiles();
      this.dispatchEvent(new Event("reloaded"));
      return true;
    } catch (error) {
      this.#status.textContent = error instanceof Error ? error.message : String(error);
      return false;
    }
  }

  #discoverImportsSoon(): void {
    if (this.#draftDiscoveryTimer) clearTimeout(this.#draftDiscoveryTimer);
    this.#draftDiscoveryTimer = setTimeout(() => {
      this.#draftDiscoveryTimer = null;
      void this.refreshFromDisk();
    }, 250);
  }

  #renderFiles(): void {
    this.#selector.replaceChildren();
    for (const file of this.drafts.files.values()) {
      const option = document.createElement("option");
      option.value = file.path;
      option.textContent = file.path.split(/[\\/]/).pop() || file.path;
      option.title = file.path;
      this.#selector.append(option);
    }
    this.#toggle.hidden = this.drafts.files.size === 0;
    if (this.drafts.files.size === 0) {
      this.#pane.hidden = true;
      this.#previewPane.classList.remove("source-open");
      return;
    }
    this.#selected = this.#selected && this.drafts.files.has(this.#selected)
      ? this.#selected
      : this.drafts.files.keys().next().value ?? null;
    this.#renderSelected();
  }

  #renderSelected(): void {
    const file = this.#selected ? this.drafts.files.get(this.#selected) : null;
    this.#selector.value = file?.path ?? "";
    if (this.#textarea.value !== (file?.text ?? "")) this.#textarea.value = file?.text ?? "";
    this.#renderStatus();
  }

  #renderStatus(): void {
    const file = this.#selected ? this.drafts.files.get(this.#selected) : null;
    this.#status.textContent = file?.conflict
      ?? (this.#saving ? "Saving…" : file && file.text !== file.savedText ? "Unsaved source" : "Saved");
    this.#saveButton.disabled = this.#saving || !this.dirty;
  }
}
