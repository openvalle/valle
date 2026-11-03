// What the tests that run Chrome share: the Chrome for Testing that tests/baselines/chrome/version
// pins, downloaded once into ~/.valle/cache/chrome (or $VALLE_CACHE_DIR/chrome), and a connection
// to its DevTools.

import { mkdir, rename, rm } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

export const root = join(import.meta.dir, "../..");
export const cache = process.env.VALLE_CACHE_DIR ?? join(homedir(), ".valle/cache");
export const version = (await Bun.file(join(root, "tests/baselines/chrome/version")).text()).trim();

const platforms: Record<string, string> = {
  "darwin-arm64": "mac-arm64",
  "darwin-x64": "mac-x64",
  "linux-arm64": "linux-arm64",
  "linux-x64": "linux64",
  "win32-x64": "win64",
};
const platform = platforms[`${process.platform}-${process.arch}`];
if (!platform) {
  throw new Error(`Chrome for Testing has no build for ${process.platform} ${process.arch}`);
}

/** The executable of a build of the pinned Chrome, downloaded into the cache the first time:
 * `chrome-headless-shell`, or the whole of Chrome, which run headless shows WebGPU canvases. */
export async function install(
  build: "chrome-headless-shell" | "chrome" = "chrome-headless-shell",
): Promise<string> {
  const dir = join(cache, "chrome", version, build);
  const executables: Record<string, string> = {
    "chrome-headless-shell":
      platform === "win64" ? "chrome-headless-shell.exe" : "chrome-headless-shell",
    chrome: platform.startsWith("mac")
      ? "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"
      : platform === "win64"
        ? "chrome.exe"
        : "chrome",
  };
  const exe = join(dir, `${build}-${platform}`, executables[build]);
  if (await Bun.file(exe).exists()) {
    return exe;
  }
  const url = `https://storage.googleapis.com/chrome-for-testing-public/${version}/${platform}/${build}-${platform}.zip`;
  console.error(`downloading Chrome for Testing ${version} from ${url}`);
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`${url}: ${response.status} ${response.statusText}`);
  }
  // Unpacked beside the cache and moved in whole, so a broken download leaves nothing behind.
  const partial = `${dir}.partial`;
  await rm(partial, { recursive: true, force: true });
  await mkdir(partial, { recursive: true });
  const zip = join(partial, "chrome.zip");
  await Bun.write(zip, response);
  const unzip =
    platform === "win64" ? ["tar", "-xf", zip, "-C", partial] : ["unzip", "-q", zip, "-d", partial];
  if ((await Bun.spawn(unzip, { stdout: "inherit", stderr: "inherit" }).exited) !== 0) {
    throw new Error(`${unzip.join(" ")} failed`);
  }
  await rm(zip);
  await rename(partial, dir);
  return exe;
}

/** The DevTools address Chrome prints once it listens. Its later output is read and dropped. */
export async function listening(stderr: ReadableStream<Uint8Array>): Promise<string> {
  const reader = stderr.getReader();
  const decoder = new TextDecoder();
  let text = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) {
      throw new Error(`Chrome stopped before it listened:\n${text}`);
    }
    text += decoder.decode(value, { stream: true });
    const found = /DevTools listening on (ws:\/\/\S+)/.exec(text);
    if (found) {
      void (async () => {
        while (!(await reader.read()).done);
      })();
      return found[1];
    }
  }
}

/** A Chrome DevTools Protocol connection: commands with their replies, and events awaited. */
export class Cdp {
  #ws: WebSocket;
  #next = 0;
  #replies = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void }>();
  #waiting: { method: string; sessionId?: string; resolve: (v: any) => void }[] = [];

  static async open(url: string): Promise<Cdp> {
    const ws = new WebSocket(url);
    await new Promise((resolve, reject) => {
      ws.onopen = resolve;
      ws.onerror = () => reject(new Error(`cannot connect to ${url}`));
    });
    return new Cdp(ws);
  }

  constructor(ws: WebSocket) {
    this.#ws = ws;
    ws.onmessage = (event) => {
      const message = JSON.parse(String(event.data));
      if (message.id !== undefined) {
        const reply = this.#replies.get(message.id);
        this.#replies.delete(message.id);
        if (message.error) {
          reply?.reject(new Error(`${message.error.message} (${message.error.code})`));
        } else {
          reply?.resolve(message.result);
        }
        return;
      }
      const i = this.#waiting.findIndex(
        (w) => w.method === message.method && w.sessionId === message.sessionId,
      );
      if (i >= 0) {
        this.#waiting.splice(i, 1)[0].resolve(message.params);
      }
    };
  }

  send(method: string, params: object = {}, sessionId?: string): Promise<any> {
    const id = ++this.#next;
    this.#ws.send(JSON.stringify({ id, method, params, sessionId }));
    return timed(
      method,
      new Promise((resolve, reject) => this.#replies.set(id, { resolve, reject })),
    );
  }

  /** The next `method` event of the session. */
  event(method: string, sessionId?: string): Promise<any> {
    return timed(
      method,
      new Promise((resolve) => this.#waiting.push({ method, sessionId, resolve })),
    );
  }
}

/** `promise`, or an error after 30 s: a page that never loads stops the run. */
function timed<T>(what: string, promise: Promise<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what}: no answer from Chrome in 30 s`)), 30_000);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}
