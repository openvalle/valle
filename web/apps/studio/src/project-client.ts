import type { Timeline } from "valle-engine";
import type { StudioBoot } from "./host.ts";

export interface AssetItem {
  path: string;
  absolutePath?: string;
  kind: string;
  alias?: string;
  status?: "ok" | "error";
  info?: { duration?: number; width?: number; height?: number; family?: string } | null;
  tags?: string[];
  notes?: Array<{ id: number; text: string; at: number | null; range: [number, number] | null }>;
  error?: { message: string };
}

export class StudioServices {
  constructor(readonly token: string) {}

  async request(domain: "project" | "library", command: object): Promise<Record<string, any>> {
    const response = await fetch(`/${domain}/execute`, {
      method: "POST", headers: { "content-type": "application/json", "x-valle-token": this.token },
      body: JSON.stringify(command),
    });
    const value = await response.json();
    if (!response.ok || value.status === "error" || value.error) {
      throw new Error(value.error?.message ?? `${domain} request failed (${response.status})`);
    }
    return value;
  }

  async importFiles(files: readonly File[]): Promise<AssetItem[]> {
    const items: AssetItem[] = [];
    for (const file of files) {
      try {
        const response = await fetch("/project/execute", {
          method: "POST", headers: { "content-type": "application/octet-stream", "x-valle-token": this.token,
            "x-valle-filename": encodeURIComponent(file.name) }, body: file,
        });
        const result = await response.json();
        if (!response.ok || result.error) throw new Error(result.error?.message ?? "Upload failed");
        items.push(...result.items.map((item: AssetItem) => ({ ...item, path: item.path ?? file.name })));
      } catch (error) {
        items.push({ path: file.name, kind: "other", status: "error", error: { message: error instanceof Error ? error.message : String(error) } });
      }
    }
    return items;
  }

  async importAssets(items: readonly AssetItem[]): Promise<AssetItem[]> {
    if (items.some((item) => !item.absolutePath)) throw new Error("Asset has no file path");
    const result = await this.request("project", { op: "add", files: items.map((item) => item.absolutePath) });
    return result.items.map((item: AssetItem,index: number) => ({ ...item, path: item.path ?? items[index]!.path }));
  }
}

/** Fail before changing a draft when any file failed. Successfully copied files remain reusable. */
export function requireImported(items: readonly AssetItem[]): void {
  const failed = items.filter((item) => item.status === "error");
  if (failed.length) throw new Error(`${failed.map((item) => `${item.path}: ${item.error?.message ?? "import failed"}`).join("; ")}. Copied files remain in the project; the Timeline was not changed.`);
}

function locator(boot: StudioBoot, projectPath: string): string {
  if (!boot.project) throw new Error("Import requires a project");
  const normalize = (path: string) => path.replaceAll("\\", "/").replace(/\/$/, "");
  const root = normalize(boot.project.root);
  const input = normalize(boot.session.input);
  const directory = input.slice(root.length + 1).split("/").slice(0, -1);
  return "../".repeat(directory.length) + projectPath;
}

/** A single undoable Timeline change. Unsupported clip types are still registered as resources. */
export function appendImportedAssets(timeline: Timeline, items: readonly AssetItem[], boot: StudioBoot, start: number): Timeline {
  requireImported(items);
  const next = structuredClone(timeline);
  const resources = next.resources ??= {};
  const visual: NonNullable<Timeline["tracks"]["visual"]>[number]["clips"] = [];
  const audio: NonNullable<Timeline["tracks"]["audio"]>[number]["clips"] = [];
  let visualStart = start, audioStart = start;
  for (const item of items) {
    const path = locator(boot, item.path);
    let alias = Object.entries(resources).find(([, value]) => value === path)?.[0];
    if (!alias) {
      const base = (item.alias ?? item.path.split("/").at(-1) ?? item.kind).replace(/[^A-Za-z0-9._-]/g, "").slice(0, 48) || item.kind;
      alias = base;
      for (let suffix = 2; alias in resources; suffix++) alias = `${base}-${suffix}`;
      resources[alias] = path;
    }
    const duration = item.info?.duration && item.info.duration > 0 ? item.info.duration : item.kind === "image" ? 5 : 3;
    if (item.kind === "audio") {
      audio.push({ src: alias, start: audioStart, duration }); audioStart += duration;
    } else if (item.kind === "video" || item.kind === "image") {
      visual.push({ kind: item.kind, src: alias, start: visualStart, duration }); visualStart += duration;
    } else if (item.kind === "component") {
      visual.push({ kind: "motion", component: alias, start: visualStart, duration, end: "hold" }); visualStart += duration;
    }
  }
  if (visual.length) (next.tracks.visual ??= []).push({ clips: visual });
  if (audio.length) (next.tracks.audio ??= []).push({ clips: audio });
  return next;
}

export interface ImportIntent {
  files?: readonly File[];
  assets?: readonly AssetItem[];
  complete: (error?: string) => void;
}
