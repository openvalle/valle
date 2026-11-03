// The local service Studio runs on: `valle motion|timeline|project studio` serves this page with
// a token in its address, which every request carries.

const token =
  new URLSearchParams(location.search).get("token") ?? sessionStorage.getItem("valle-token") ?? "";
sessionStorage.setItem("valle-token", token);

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

/** `fetch` of the service, with the token. */
export function request(path: string, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers);
  headers.set("x-valle-token", token);
  return fetch(path, { ...init, headers });
}

async function json<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await request(path, init);
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

/** The same, with the token in it, for a link the browser follows. */
export function link(path: string): string {
  return `${address(path)}?token=${token}`;
}

export const service = {
  session: () => json<Session>("/api/session"),
  open: (folder: string) => post<Session>("/api/open", { folder }),
  list: async () => (await json<{ files: Listed[] }>("/api/list")).files,

  async read(path: string): Promise<Read | undefined> {
    const response = await request(address(path));
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
    const response = await request(address(path), {
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
    post<{ text: string; layout: Layout }>("/api/edit", { text, op }),

  /** `valle project <args> --json` on the project open. */
  project: (args: string[]) => post<Record<string, any>>("/api/project", { args }),

  /** Starts a render; it says how it goes as `render` events with its id. */
  render: async (args: string[]) => (await post<{ id: number }>("/api/render", { args })).id,

  /** The service's events: `change` of a file read, and `render`. */
  events(on: (event: Record<string, any>) => void): EventSource {
    const source = new EventSource(`/api/events?token=${token}`);
    source.onmessage = (message) => on(JSON.parse(message.data));
    return source;
  },
};

/** What the player fetches through: the service's files with the token, and other origins'
 * through the service, which needs no CORS from them. */
export function fetching(url: string): Promise<Response> {
  const at = new URL(url, location.href);
  if (at.origin === location.origin) {
    return request(at.pathname + at.search);
  }
  return request(`/api/url?u=${encodeURIComponent(at.href)}`);
}
