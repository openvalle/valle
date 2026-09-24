import type { SourceFile, StudioHost, WriteSourceResult } from "./host.ts";

export interface SourceDraft {
  path: string;
  text: string;
  savedText: string;
  baseDigest: string;
  conflict: string | null;
}

export interface SourceSaveReport {
  saved: Record<string, string>;
  failed: { path: string; message: string } | null;
}

/** Exact source text and disk digest stay separate from compiled Motion output. */
export class SourceDrafts {
  readonly files = new Map<string, SourceDraft>();
  readonly unavailable: Array<{ path: string; message: string }> = [];

  constructor(files: SourceFile[]) {
    for (const file of files) {
      if (file.status === "ok") {
        this.files.set(file.path, {
          path: file.path,
          text: file.text,
          savedText: file.text,
          baseDigest: file.baseDigest,
          conflict: null,
        });
      } else {
        this.unavailable.push({ path: file.path, message: file.message });
      }
    }
  }

  get dirty(): boolean {
    return [...this.files.values()].some((file) => file.text !== file.savedText);
  }

  confirmedDigests(): Record<string, string> {
    return Object.fromEntries([...this.files.values()].map((file) => [file.path, file.baseDigest]));
  }

  snapshot(): Record<string, string> {
    return Object.fromEntries([...this.files.values()].map((file) => [file.path, file.text]));
  }

  restore(snapshot: Record<string, string>): void {
    for (const file of this.files.values()) {
      if (Object.hasOwn(snapshot, file.path)) file.text = snapshot[file.path]!;
    }
  }

  edit(path: string, text: string): void {
    const file = this.files.get(path);
    if (!file) throw new Error(`Source is not loaded: ${path}`);
    file.text = text;
    file.conflict = null;
  }

  /** Merge a fresh disk manifest without replacing any unsaved editor text. */
  reconcile(files: SourceFile[]): boolean {
    let changed = false;
    for (const incoming of files) {
      const file = this.files.get(incoming.path);
      if (incoming.status === "error") {
        if (file) {
          if (file.conflict !== incoming.message) changed = true;
          file.conflict = incoming.message;
        } else {
          const unavailable = this.unavailable.find((entry) => entry.path === incoming.path);
          if (unavailable) {
            if (unavailable.message !== incoming.message) changed = true;
            unavailable.message = incoming.message;
          } else {
            this.unavailable.push({ path: incoming.path, message: incoming.message });
            changed = true;
          }
        }
        continue;
      }
      const unavailableIndex = this.unavailable.findIndex((entry) => entry.path === incoming.path);
      if (unavailableIndex >= 0) {
        this.unavailable.splice(unavailableIndex, 1);
        changed = true;
      }
      if (!file) {
        this.files.set(incoming.path, {
          path: incoming.path,
          text: incoming.text,
          savedText: incoming.text,
          baseDigest: incoming.baseDigest,
          conflict: null,
        });
        changed = true;
        continue;
      }
      if (incoming.baseDigest === file.baseDigest) {
        if (file.conflict !== null) changed = true;
        file.conflict = null;
        continue;
      }
      if (incoming.text === file.text) {
        // This also confirms a write whose response was still in flight.
        file.savedText = incoming.text;
        file.baseDigest = incoming.baseDigest;
        file.conflict = null;
        changed = true;
      } else if (file.text === file.savedText) {
        file.text = incoming.text;
        file.savedText = incoming.text;
        file.baseDigest = incoming.baseDigest;
        file.conflict = null;
        changed = true;
      } else {
        if (file.conflict !== "Source changed outside Studio") changed = true;
        file.conflict = "Source changed outside Studio";
      }
    }
    return changed;
  }

  async saveDirty(host: StudioHost): Promise<SourceSaveReport> {
    const submitted = [...this.files.values()]
      .filter((file) => file.text !== file.savedText)
      .map((file) => ({ path: file.path, text: file.text, baseDigest: file.baseDigest }));
    const saved: Record<string, string> = {};
    for (const input of submitted) {
      let result: WriteSourceResult;
      try {
        result = await host.writeSourceFile(input);
      } catch (error) {
        // A lost response may still mean the bytes reached disk. Read the real
        // file before deciding whether retrying could overwrite another edit.
        try {
          const files = await host.loadSourceFiles();
          const current = files.find((file) => file.path === input.path);
          if (current?.status === "ok" && current.text === input.text) {
            result = { status: "unchanged", digest: current.baseDigest };
          } else {
            const message = current?.status === "ok" && current.baseDigest !== input.baseDigest
              ? "Source changed outside Studio"
              : error instanceof Error ? error.message : String(error);
            const file = this.files.get(input.path)!;
            file.conflict = message;
            return { saved, failed: { path: input.path, message } };
          }
        } catch {
          const message = error instanceof Error ? error.message : String(error);
          this.files.get(input.path)!.conflict = message;
          return { saved, failed: { path: input.path, message } };
        }
      }
      const file = this.files.get(input.path)!;
      if (result.status === "saved" || result.status === "unchanged") {
        file.savedText = input.text;
        file.baseDigest = result.digest;
        file.conflict = null;
        saved[input.path] = result.digest;
      } else {
        const message = result.status === "conflict"
          ? "Source changed outside Studio"
          : "Source changed again after this save";
        file.conflict = message;
        return { saved, failed: { path: input.path, message } };
      }
    }
    return { saved, failed: null };
  }
}
