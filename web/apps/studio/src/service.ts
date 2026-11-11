// The local service `valle motion|timeline|project studio` serves alongside this page.

/** What is open: a scene, a timeline, a project, or nothing yet, on the projects' home. */
export type Session = {
  kind: "motion" | "timeline" | "project" | "home";
  entry: string | null;
  root: string | null;
  name: string | null;
  timelines?: string[];
  recent?: { path: string; opened: number }[];
};

/** A file in the folder open. */
export type Listed = { path: string; size: number; modified: number };

/** Where a timeline's clips are, as the service lays it out. */
export type Layout = {
  duration: number;
  fps: number;
  width: number;
  height: number;
  tracks: { id: string; kind: string; at: string; clips: Clip[] }[];
};

export type Clip = {
  id: string;
  named: boolean;
  at: string;
  name: string | null;
  start: number;
  duration: number;
  enabled: boolean;
  anchor: string | null;
  kind: string;
};

/** A file as read: its text, and its SHA-256, which a write says it replaces. */
export type Read = { text: string; sha256: string };

/** What a write of a file that changed since it was read answers. */
export class Conflict extends Error {
  constructor(
    readonly path: string,
    readonly sha256: string | null,
  ) {
    super(`${path} changed on disk`);
  }
}

async function json<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, init);
  const value = await response.json();
  if (!response.ok || value.status === "error") {
    throw new Error(value.message ?? `${path}: ${response.status}`);
  }
  return value as T;
}

function post<T>(path: string, body: unknown): Promise<T> {
  return json<T>(path, { method: "POST", body: JSON.stringify(body) });
}

/** A file's address on the service, which a link or a player reads. */
export function address(path: string): string {
  return `/api/files/${path.split("/").map(encodeURIComponent).join("/")}`;
}

export const service = {
  session: () => json<Session>("/api/session"),
  open: (folder: string) => post<Session>("/api/open", { folder }),
  list: async () => (await json<{ files: Listed[] }>("/api/list")).files,
  saveFile: async (name: string) =>
    (await post<{ path: string | null }>("/api/save-file", { name })).path,

  async read(path: string): Promise<Read | undefined> {
    const response = await fetch(address(path));
    if (response.status === 404) {
      return undefined;
    }
    if (!response.ok) {
      throw new Error(`${path}: ${await response.text()}`);
    }
    return { text: await response.text(), sha256: response.headers.get("x-valle-sha256")! };
  },

  /** Writes `text` over the file as it was read, `base`, or as a new file with none. */
  async write(path: string, text: string, base: string | null): Promise<string> {
    const response = await fetch(address(path), {
      method: "PUT",
      headers: { "x-valle-base": base ?? "none" },
      body: text,
    });
    const value = await response.json();
    if (response.status === 409) {
      throw new Conflict(path, value.sha256);
    }
    if (!response.ok) {
      throw new Error(value.message ?? `${path}: ${response.status}`);
    }
    return value.sha256 as string;
  },

  /** A timeline's text with `op` done, and where its clips are then. */
  edit: (text: string, op?: unknown) =>
    post<{ text: string; layout: Layout; summary: string }>("/api/edit", { text, op }),

  /** `valle project <args> --json` on the project open. */
  project: (args: string[]) => post<Record<string, any>>("/api/project", { args }),

  /** Starts a render; it says how it goes as `render` events with its id. */
  render: async (args: string[], overwrite = false) =>
    (await post<{ id: number }>("/api/render", { args, overwrite })).id,

  /** The service's events: `change` of a file read, and `render`. */
  events(on: (event: Record<string, any>) => void): EventSource {
    const source = new EventSource("/api/events");
    source.onmessage = (message) => on(JSON.parse(message.data));
    return source;
  },
};

/** What the player fetches through: the service's files, and other origins'
 * through the service, which needs no CORS from them. */
export function fetching(url: string): Promise<Response> {
  const at = new URL(url, location.href);
  if (at.origin === location.origin) {
    return at.pathname.startsWith("/api/")
      ? fetch(at.pathname + at.search)
      : fetch(`/api/linked?file=${encodeURIComponent(decodeURIComponent(at.pathname))}`);
  }
  // Windows absolute paths are parsed by URL as a drive-letter scheme.
  if (/^[a-z]:$/i.test(at.protocol)) {
    return fetch(`/api/linked?file=${encodeURIComponent(decodeURIComponent(at.href))}`);
  }
  return fetch(`/api/url?u=${encodeURIComponent(at.href)}`);
}
